use super::Handler;

pub struct FfmpegHandler;

impl Handler for FfmpegHandler {
    fn filter(&self, output: &str, _args: &[String]) -> String {
        let lines: Vec<&str> = output.lines().collect();
        if lines.len() < 5 {
            return output.to_string();
        }

        let mut result: Vec<String> = Vec::new();
        for line in &lines {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            // Drop frame= progress lines (rewritten each second)
            if t.starts_with("frame=") {
                continue;
            }
            // Drop size= progress lines
            if t.starts_with("size=") {
                continue;
            }
            result.push(line.to_string());
        }

        if result.is_empty() {
            output.to_string()
        } else {
            result.join("\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::Handler;

    fn handler() -> FfmpegHandler {
        FfmpegHandler
    }

    #[test]
    fn frame_progress_lines_dropped() {
        let output = "\
ffmpeg version 6.0 Copyright (c) 2000-2023 the FFmpeg developers
Input #0, mov,mp4,m4a, from 'input.mp4':
  Duration: 00:01:30.00, start: 0.000000, bitrate: 4096 kb/s
    Stream #0:0: Video: h264, yuv420p, 1920x1080, 30 fps
    Stream #0:1: Audio: aac, 48000 Hz, stereo
Output #0, mp4, to 'output.mp4':
    Stream #0:0: Video: h264, 1920x1080
frame=  100 fps= 30 q=28.0 size=    1024kB time=00:00:03.33 bitrate=2516.6kbits/s speed=1x
frame=  200 fps= 30 q=28.0 size=    2048kB time=00:00:06.67 bitrate=2516.6kbits/s speed=1x
frame= 2700 fps= 30 q=28.0 size=   45056kB time=00:01:30.00 bitrate=4096.0kbits/s speed=1x
video:44020kB audio:1418kB subtitle:0kB other streams:0kB global headers:0kB muxing overhead: 0.040%
";
        let result = handler().filter(output, &[]);
        assert!(!result.contains("frame="), "should drop frame= lines");
        assert!(result.contains("Duration:"), "should keep input info");
        assert!(result.contains("video:44020kB"), "should keep final size line");
        assert!(result.contains("1920x1080"), "should keep resolution info");
    }

    #[test]
    fn short_output_passes_through() {
        let output = "ffmpeg version 6.0\n";
        let result = handler().filter(output, &[]);
        assert_eq!(result, output);
    }

    #[test]
    fn size_progress_lines_dropped() {
        let output = "\
ffmpeg version 6.0
Input #0, mp3, from 'audio.mp3':
  Duration: 00:03:45.00
Output #0, wav, to 'audio.wav':
size=    1024kB time=00:00:30.00 bitrate= 279.6kbits/s speed=60.0x
size=    5120kB time=00:03:45.00 bitrate= 186.4kbits/s speed=60.0x
video:0kB audio:39375kB subtitle:0kB
";
        let result = handler().filter(output, &[]);
        assert!(!result.contains("size=    1024kB"), "should drop size= progress lines");
        assert!(result.contains("audio:39375kB"), "should keep final summary line");
    }
}
