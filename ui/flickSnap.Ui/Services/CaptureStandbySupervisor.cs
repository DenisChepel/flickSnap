using System;
using System.Diagnostics;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Avalonia.Threading;
using FlickSnap.Config;
using FlickSnap.Ipc;
using FlickSnap.Util;
using IpcCaptureMode = FlickSnap.Ipc.CaptureMode;

namespace FlickSnap.UI
{
    /// <summary>
    /// Runs the capturer in its lightweight request-waiting standby mode over the <c>capture</c>
    /// catalog (HELPER_PROTOCOL.md §4.5). The screenshot hotkeys stay with the shell: a press,
    /// like a tray click, reaches the standby through <see cref="TryCapture"/>.
    /// Exists only while warm capture is in effect: created and disposed by
    /// <c>App.ConfigureCaptureStandby</c>, which owns the choice between this and the classic
    /// one-shot path. When standby cannot work — missing permission or binary,
    /// permission-revoked exit, repeated crashes — the supervisor calls the fallback delegate
    /// exactly once and stops; the shell then reverts to one-shot capture for the rest of its
    /// process lifetime without touching the saved setting.
    /// </summary>
    internal sealed class CaptureStandbySupervisor : IDisposable
    {
        private const int MaxConsecutiveFailures = 3;

        /// <summary>A process that has been running this long forgives earlier crashes:
        /// without this, three unrelated crashes weeks apart (say one per GPU driver
        /// update) would silently degrade a long-lived session to one-shot forever.
        /// Watchdog kills are exempt — see the failure accounting in SuperviseAsync.</summary>
        private static readonly TimeSpan StableUptime = TimeSpan.FromMinutes(10);

        /// <summary>How long the overlay may stay open before the process is presumed hung
        /// and killed (the crash path then restarts or falls back). Generous on purpose —
        /// people do sit in the overlay — because a false positive costs a capture, while a
        /// true hang would otherwise swallow tray clicks and block updates forever.</summary>
        private static readonly TimeSpan OverlayHangTimeout = TimeSpan.FromMinutes(30);

        private static CaptureStandbySupervisor _current;

        /// <summary>True while the standby capturer's overlay is on screen (or an accepted
        /// request is about to put it there) — merged into ScreenCapturePage.IsCaptureActive
        /// for the IdleMonitor update gate.</summary>
        public static bool IsOverlayOpen =>
            Volatile.Read(ref _current) is { } current && current.OverlayInProgress;

        private enum CaptureState { Waiting, Ready, Requested, Open }

        private CaptureState State => (CaptureState)Volatile.Read(ref _state);
        private bool OverlayInProgress => State is CaptureState.Requested or CaptureState.Open;

        private void Transition(CaptureState from, CaptureState to) =>
            Interlocked.CompareExchange(ref _state, (int)to, (int)from);

        private readonly CancellationTokenSource _stop = new();
        private readonly object _stdinGate = new();
        private Action _onFallback;
        private Action _returnFocus;
        private HelperClient<CaptureEvent, CaptureCommand> _client;
        private string _lastSettings;
        private int _state;
        private long _overlayOpenedAtTicks;
        private int _watchdogKilled;

        public CaptureStandbySupervisor(Action onFallback)
        {
            _onFallback = onFallback;
            _current = this;
            SettingsRoot.Current.Capture.PropertyChanged += OnSettingsChanged;
            SettingsRoot.Current.General.PropertyChanged += OnSettingsChanged;
            // the recording, upload and share modes decide the strip's VIDEO, UPLOAD and SHARE
            // buttons (--no-video / --no-upload / --no-share); everything else on those pages is
            // deduped away by the fingerprint in SendSettings.
            SettingsRoot.Current.Recording.PropertyChanged += OnSettingsChanged;
            SettingsRoot.Current.Uploads.PropertyChanged += OnSettingsChanged;
            SettingsRoot.Current.ShareRegion.PropertyChanged += OnSettingsChanged;
            _ = Task.Run(RunAsync);
        }

        /// <summary>
        /// Routes a shell-initiated capture (hotkey, tray, menu) to the waiting standby process.
        /// True: handled, either sent or dropped because an overlay is already up. False: standby cannot take it right now
        /// (no supervisor, faulted, or between a capture and the next <c>ready</c>), so the
        /// caller must run the classic one-shot capture instead. Nothing is ever queued: a request that cannot be
        /// satisfied promptly is either dropped as a duplicate or taken over by the fallback.
        /// </summary>
        public static bool TryCapture(CaptureMode mode, Action returnFocus)
        {
            var current = Volatile.Read(ref _current);
            return current != null && current.TrySendCapture(mode, returnFocus);
        }

        private bool TrySendCapture(CaptureMode mode, Action returnFocus)
        {
            var command = new CaptureCommand.Capture(mode switch
            {
                CaptureMode.Screen => IpcCaptureMode.Screen,
                CaptureMode.Window => IpcCaptureMode.Window,
                _ => IpcCaptureMode.Region,
            });
            lock (_stdinGate)
            {
                if (OverlayInProgress)
                    return true;
                if (State != CaptureState.Ready || !SendSettingsLocked())
                    return false;
                Volatile.Write(ref _overlayOpenedAtTicks, DateTime.UtcNow.Ticks);
                Volatile.Write(ref _returnFocus, returnFocus);
                Volatile.Write(ref _state, (int)CaptureState.Requested);
                if (SendLocked(command, grantForeground: true))
                    return true;
                Interlocked.Exchange(ref _returnFocus, null);
                Transition(CaptureState.Requested, CaptureState.Waiting);
            }
            return false;
        }

        private void OnSettingsChanged(object sender, System.ComponentModel.PropertyChangedEventArgs e) => SendSettings();

        /// <summary>Backstop wrapper: nothing may escape the supervise loop silently — an
        /// unread stdout pipe would hang the child and lose captures with no one watching.</summary>
        private async Task RunAsync()
        {
            try
            {
                await SuperviseAsync();
            }
            catch (Exception ex)
            {
                try { SentryConfig.CaptureHandled(ex, "capture.standby-supervisor-crash"); }
                catch { }
                try { Volatile.Read(ref _client)?.Kill(); }
                catch { }
                Fallback("supervisor crashed: " + ex.Message);
            }
        }

        private async Task SuperviseAsync()
        {
            int failures = 0;
            while (!_stop.IsCancellationRequested)
            {
                string sessionDir = null;
                HelperClient<CaptureEvent, CaptureCommand> client = null;
                using var processExited = CancellationTokenSource.CreateLinkedTokenSource(_stop.Token);
                try
                {
                    if (!MacPermissions.HasScreenRecording)
                    {
                        // The one-shot path owns the whole permission conversation (prompt,
                        // dialog, retry per attempt); standby must not spawn a capturer that
                        // can only see a black desktop.
                        Fallback("screen recording permission not granted");
                        return;
                    }

                    var binary = HelperLocator.Find(HelperNames.Capture);
                    if (binary == null)
                    {
                        // one-shot shows its "binary missing" dialog on the next attempt
                        Fallback("capture binary not found");
                        return;
                    }

                    // The open stdin is the child's parent-lifetime lease: if FlickSnap.Ui terminates
                    // for any reason, EOF tells the standby event loop to exit.
                    // Not a tree kill: on Windows the browser an OCR search opens is this process's
                    // child when no browser was already running, and quitting FlickSnap, turning warm
                    // capture off or the hang watchdog must not close the user's browser (§7.4).
                    client = await HelperClient<CaptureEvent, CaptureCommand>.StartAsync(HelperSpecs.Capture, binary,
                        CaptureArguments.BuildStandby(PathConstants.SessionData, SettingsRoot.Current,
                            SettingsRoot.Current.General.LastSavePath),
                        new HelperStartOptions { KillEntireTree = false });
                    var startedUtc = DateTime.UtcNow;
                    lock (_stdinGate)
                    {
                        _client = client;
                        _lastSettings = null;
                    }
                    // Dekker pair with Dispose (Cancel then Kill-via-_client): a Dispose that ran
                    // before the publish above saw null and killed nothing, so re-check here;
                    // otherwise this child leaks alive.
                    if (_stop.IsCancellationRequested)
                    {
                        client.Kill();
                        return;
                    }

                    _ = WatchOverlayHangAsync(client, processExited.Token);
                    await foreach (var ev in client.Events.ReadAllAsync())
                    {
                        switch (ev)
                        {
                            case CaptureEvent.Ready:
                                Transition(CaptureState.Waiting, CaptureState.Ready);
                                // off the reader: a write that blocks on a full stdin pipe must
                                // never stall the loop that drains the child's stdout
                                _ = Task.Run(SendSettings);
                                break;

                            case CaptureEvent.Session session:
                                sessionDir = session.Dir;
                                Volatile.Write(ref _overlayOpenedAtTicks, DateTime.UtcNow.Ticks);
                                Volatile.Write(ref _state, (int)CaptureState.Open);
                                IdleMonitor.NotifyCaptureActivity();
                                break;

                            case CaptureEvent.CaptureFinished finished:
                                sessionDir = null;
                                failures = 0;
                                var returnFocus = Interlocked.Exchange(ref _returnFocus, null);
                                Transition(CaptureState.Open, CaptureState.Waiting);
                                IdleMonitor.NotifyCaptureActivity();
                                var dir = finished.Dir;
                                Dispatcher.UIThread.Post(async () => await ScreenCapturePage.ProcessStandbySessionAsync(dir, returnFocus));
                                break;

                            case CaptureEvent.Error { Fatal: false } error:
                                Debug.WriteLine($"Standby capturer refused a request: {error.Code}: {error.Message}");
                                // Only a failed session creation rejects a pending capture.
                                // A settings rejection can arrive while a capture is requested.
                                if (error.Code == IpcErrorCodes.IoFailed && sessionDir == null)
                                {
                                    Interlocked.Exchange(ref _returnFocus, null);
                                    Transition(CaptureState.Requested, CaptureState.Waiting);
                                }
                                break;
                        }
                    }

                    var outcome = await client.Completion;
                    if (_stop.IsCancellationRequested)
                        return;

                    // A permission that went away, or a hook that could not come up at all, is
                    // not a crash and will not get better by restarting: one-shot mode owns that
                    // conversation from here.
                    if (outcome.Failure == HelperFailure.NoScreenPermission
                        || outcome.Error?.Code is IpcErrorCodes.Permission or IpcErrorCodes.InitFailed)
                    {
                        Fallback("standby capturer cannot run: " + outcome.Describe());
                        return;
                    }

                    // The standby process never exits on its own while the shell lives, so any
                    // exit here is a failure of some kind. Uptime does not forgive a watchdog
                    // kill: a capturer that hangs on every capture always runs "stably" past the
                    // threshold first, and forgiving it would trade permanent 30-minute hangs
                    // against ever reaching the fallback.
                    if (Interlocked.Exchange(ref _watchdogKilled, 0) == 0 && DateTime.UtcNow - startedUtc > StableUptime)
                        failures = 0;
                    failures++;

                    Debug.WriteLine($"Standby capturer ended: {outcome.Describe()}");
                    ReportCrash(outcome, failures, TryReadCaptureLog(sessionDir));
                }
                catch (OperationCanceledException) when (_stop.IsCancellationRequested)
                {
                    return;
                }
                catch (Exception ex)
                {
                    failures++;
                    Debug.WriteLine("Standby capturer failed: " + ex);
                    ex.Data["consecutive_failures"] = failures;
                    SentryConfig.CaptureHandled(ex, "capture.standby-supervisor");
                }
                finally
                {
                    processExited.Cancel();
                    lock (_stdinGate)
                    {
                        _client = null;
                        Volatile.Write(ref _state, (int)CaptureState.Waiting);
                        Interlocked.Exchange(ref _returnFocus, null);
                    }
                    if (client != null)
                        await client.DisposeAsync();
                    // any session not announced as finished is a partial the shell owns
                    // deleting, on every exit path (crash, cancellation, escape)
                    if (sessionDir != null)
                        CaptureSessionDispatcher.DeleteSessionDir(sessionDir);
                }

                if (failures >= MaxConsecutiveFailures)
                {
                    Fallback($"{failures} consecutive standby failures");
                    return;
                }
                try
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(500 * failures), _stop.Token);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
            }
        }

        /// <summary>The only self-heal for a hung-but-alive capturer: if the overlay has
        /// supposedly been open far longer than any real capture, kill the process — the
        /// normal crash handling then cleans up, restarts, or falls back. Without this, a
        /// wedged child swallows every tray click and blocks updates until app restart.
        /// Two consecutive expired samples protect against a sample that straddles captures.</summary>
        private async Task WatchOverlayHangAsync(HelperClient<CaptureEvent, CaptureCommand> client, CancellationToken token)
        {
            try
            {
                int expiredSamples = 0;
                while (!token.IsCancellationRequested)
                {
                    await Task.Delay(TimeSpan.FromMinutes(1), token);
                    var openedAt = new DateTime(Volatile.Read(ref _overlayOpenedAtTicks), DateTimeKind.Utc);
                    if (!OverlayInProgress || DateTime.UtcNow - openedAt < OverlayHangTimeout)
                    {
                        expiredSamples = 0;
                        continue;
                    }
                    if (++expiredSamples < 2)
                        continue;
                    Debug.WriteLine("Standby capturer overlay unresponsive; killing process");
                    // kill first: a hang must be broken even where telemetry itself throws
                    Interlocked.Exchange(ref _watchdogKilled, 1);
                    try { client.Kill(); }
                    catch { }
                    try
                    {
                        SentryConfig.CaptureHandled(
                            new TimeoutException($"Standby capturer overlay open for over {OverlayHangTimeout.TotalMinutes:0} minutes; killed"),
                            "capture.standby-overlay-hang");
                    }
                    catch { }
                    return;
                }
            }
            catch
            {
                // best-effort watchdog: cancellation (normal) or a disposed token source
                // racing process exit — either way the supervise loop owns cleanup
            }
        }

        /// <summary>Hands capture duty back to the shell's one-shot path, at most once.</summary>
        private void Fallback(string reason)
        {
            Debug.WriteLine("Standby capturer falling back to one-shot capture: " + reason);
            var callback = Interlocked.Exchange(ref _onFallback, null);
            try
            {
                callback?.Invoke();
            }
            catch (Exception ex)
            {
                Debug.WriteLine("Standby fallback callback failed: " + ex);
            }
        }

        private void SendSettings()
        {
            lock (_stdinGate)
            {
                if (State == CaptureState.Ready)
                    SendSettingsLocked();
            }
        }

        private bool SendSettingsLocked()
        {
            try
            {
                // Refresh under the same lock as capture so a hotkey can never
                // overtake settings changes made while the previous overlay was open.
                var args = CaptureArguments.BuildStandby(PathConstants.SessionData, SettingsRoot.Current,
                    SettingsRoot.Current.General.LastSavePath);
                var fingerprint = JsonSerializer.Serialize(args);
                if (fingerprint == _lastSettings)
                    return true;
                if (!SendLocked(new CaptureCommand.Settings(args)))
                {
                    Transition(CaptureState.Ready, CaptureState.Waiting);
                    return false;
                }
                _lastSettings = fingerprint;
                return true;
            }
            catch (Exception ex)
            {
                Debug.WriteLine("Failed to send standby settings: " + ex);
                return false;
            }
        }

        private bool SendLocked(CaptureCommand command, bool grantForeground = false)
        {
            try
            {
                if (_client is not { } client)
                    return false;
                // The overlay needs keyboard focus immediately (Esc / shortcuts), and a
                // process poked from the background is denied SetForegroundWindow by the
                // foreground lock. Best-effort, exactly like the one-shot spawn: it works
                // because the user just clicked our tray/menu, so we hold the rights.
                if (grantForeground)
                    HelperProcess.GrantForeground(client.ProcessId);
                return client.Send(command);
            }
            catch (Exception ex)
            {
                Debug.WriteLine("Failed to write standby IPC: " + ex);
                return false;
            }
        }

        private static string TryReadCaptureLog(string sessionDir)
        {
            try
            {
                if (String.IsNullOrEmpty(sessionDir))
                    return null;
                var path = Path.Combine(sessionDir, "capture.log");
                return File.Exists(path) ? File.ReadAllText(path) : null;
            }
            catch { return null; }
        }

        private static void ReportCrash(HelperOutcome<CaptureEvent> outcome, int failures, string captureLog)
        {
            var crash = new InvalidOperationException(failures >= MaxConsecutiveFailures
                ? "Standby capturer disabled after consecutive crashes"
                : "Standby capturer exited unexpectedly");
            crash.Data["exit_code"] = outcome.ExitCode;
            crash.Data["failure"] = outcome.Failure.ToString();
            crash.Data["error_code"] = outcome.Error?.Code;
            crash.Data["consecutive_failures"] = failures;
            SentryConfig.AttachProcessLog(crash, ("error", outcome.Error?.Message), ("stderr", outcome.StderrTail),
                ("capture.log", captureLog));
            SentryConfig.CaptureHandled(crash, failures >= MaxConsecutiveFailures
                ? "capture.standby-consecutive-crashes"
                : "capture.standby-process-crash");
        }

        public void Dispose()
        {
            SettingsRoot.Current.Capture.PropertyChanged -= OnSettingsChanged;
            SettingsRoot.Current.General.PropertyChanged -= OnSettingsChanged;
            SettingsRoot.Current.Recording.PropertyChanged -= OnSettingsChanged;
            SettingsRoot.Current.Uploads.PropertyChanged -= OnSettingsChanged;
            SettingsRoot.Current.ShareRegion.PropertyChanged -= OnSettingsChanged;
            Interlocked.CompareExchange(ref _current, null, this);
            _onFallback = null;
            _stop.Cancel();
            // Kill WITHOUT _stdinGate: a writer stuck in SendLocked on a full pipe would
            // hold the lock, and this kill is the only thing that can unstick it; taking
            // the lock here would deadlock the UI thread. The runner observes the
            // kill/cancel and winds itself down (waiting for it here would stall the UI
            // thread during shutdown or a warm-setting toggle) and disposes the client in its
            // finally.
            try { Volatile.Read(ref _client)?.Kill(); }
            catch { }
        }
    }
}
