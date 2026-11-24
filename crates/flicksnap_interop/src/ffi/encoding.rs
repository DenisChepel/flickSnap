//! The H.264 quality mapping (README section 4.10) over `flicksnap_shared::encoding`, the one
//! implementation the recorder's encoder settings use too.

use crate::error::Error;
use crate::ffi::status::FLICKSNAP_OK;
use crate::{panic, util};
use flicksnap_shared::encoding;

/// One crf mapped onto every encoder (16 bytes): x264's crf (and AMF's qp), NVENC's constant-quality
/// target, VideoToolbox's quality slider, and the Intel-Mac VideoToolbox average bitrate in kbps.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlickSnapH264Quality {
    pub crf: i32,
    pub nvenc_cq: i32,
    pub vt_quality: i32,
    pub vt_bitrate_kbps: i32,
}

/// Maps `crf` (0 best to 51 worst, x264's scale) onto NVENC CQ and VideoToolbox quality and bitrate
/// for a `width` x `height` output at `fps_num` / `fps_den` frames per second (the size and rate
/// only matter to the bitrate). `out` is written only on `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `out`, a crf outside 0 to 51, or a size or
/// rate component that is not positive.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_encoding_h264_quality(
    crf: i32,
    width: i32,
    height: i32,
    fps_num: i32,
    fps_den: i32,
    out: *mut FlickSnapH264Quality,
) -> i32 {
    panic::guard("flicksnap_encoding_h264_quality", || {
        let out = util::out(out, "out")?;
        let q = encoding::h264_quality(crf, width, height, fps_num, fps_den).map_err(|e| Error::invalid_arg(e.to_string()))?;
        *out = FlickSnapH264Quality {
            crf: q.crf,
            nvenc_cq: q.nvenc_cq,
            vt_quality: q.video_toolbox_quality,
            vt_bitrate_kbps: q.video_toolbox_bitrate_kbps,
        };
        Ok(FLICKSNAP_OK)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_are_pinned() {
        assert_eq!(size_of::<FlickSnapH264Quality>(), 16);
        assert_eq!(offset_of!(FlickSnapH264Quality, crf), 0);
        assert_eq!(offset_of!(FlickSnapH264Quality, nvenc_cq), 4);
        assert_eq!(offset_of!(FlickSnapH264Quality, vt_quality), 8);
        assert_eq!(offset_of!(FlickSnapH264Quality, vt_bitrate_kbps), 12);
    }
}
