// SPDX-License-Identifier: Apache-2.0
// whisper-helper — transcribes a raw 16 kHz mono f32 PCM file with a
// whisper.cpp ggml model and prints the transcript to stdout.
//
// Built from this same crate as a second binary (it links whisper.cpp only,
// which keeps the ggml symbols of llama.cpp and whisper.cpp from colliding
// in one process). Invoked one-shot by the app's local-voice pipeline —
// this is not a server.

use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: whisper-helper <model.ggml.bin> <pcm-f32le-file>");
        std::process::exit(2);
    }
    if let Err(e) = run(&args[1], &args[2]) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

fn run(model: &str, pcm: &str) -> Result<(), String> {
    let bytes = std::fs::read(Path::new(pcm)).map_err(|e| format!("read pcm: {}", e))?;
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if samples.len() < 1600 {
        return Err("recording too short".to_string());
    }

    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|e| format!("load model: {}", e))?;
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(4)
            .clamp(1, 16),
    );
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_language(Some("auto"));
    state
        .full(params, &samples)
        .map_err(|e| format!("whisper: {}", e))?;

    let mut out = String::new();
    for segment in state.as_iter() {
        out.push_str(&segment.to_str_lossy().map_err(|e| e.to_string())?);
    }
    print!("{}", out.trim());
    Ok(())
}
