// The flicksnap_obs crate is a plain safe wrapper: it needs no link inputs of its own
// (flicksnap_obs_sys emits those). This script exists only so the crate's *test harness*
// can launch on macOS and Linux: the harness links libobs via flicksnap_obs_sys, and
// cargo does not propagate `cargo:rustc-link-arg` from flicksnap_obs_sys to dependents.
fn main() {
    flicksnap_build_support::link_obs_rpaths();
}
