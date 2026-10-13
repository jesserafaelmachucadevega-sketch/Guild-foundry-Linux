import { invoke } from '../../lib/api';
import { convertFileSrc } from '@tauri-apps/api/core';

const SPEECH_KEY = 'media.speech_enabled';

let currentAudio: HTMLAudioElement | null = null;

export function stopSpeech() {
  if (currentAudio) {
    currentAudio.pause();
    currentAudio.src = '';
    currentAudio = null;
  }
}

/** Persisted mute/unmute for agent speech output. */
export async function getSpeechEnabled(): Promise<boolean> {
  try {
    const v = await invoke<string | null>('settings_get', { key: SPEECH_KEY });
    return v === 'true';
  } catch {
    return false;
  }
}

export async function setSpeechEnabled(on: boolean): Promise<void> {
  try {
    await invoke('settings_set', { key: SPEECH_KEY, value: on ? 'true' : 'false' });
  } catch {
    /* settings unavailable — keep in-memory default */
  }
  if (!on) stopSpeech();
}

/** Convert a recorded blob to 16 kHz mono Float32 (whisper's input format). */
async function blobToPcm16k(blob: Blob): Promise<Float32Array> {
  const arrayBuf = await blob.arrayBuffer();
  const ctx = new AudioContext();
  try {
    const decoded = await ctx.decodeAudioData(arrayBuf);
    const targetRate = 16000;
    const length = Math.max(1, Math.ceil(decoded.duration * targetRate));
    const offline = new OfflineAudioContext(1, length, targetRate);
    const source = offline.createBufferSource();
    source.buffer = decoded;
    source.connect(offline.destination);
    source.start();
    const rendered = await offline.startRendering();
    return rendered.getChannelData(0);
  } finally {
    void ctx.close();
  }
}

function float32ToBase64(pcm: Float32Array): string {
  const bytes = new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength);
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

async function localSttAvailable(): Promise<boolean> {
  try {
    const st = await invoke<{ whisper_available?: boolean }>('media_local_voice_status');
    return !!st?.whisper_available;
  } catch {
    return false;
  }
}

/**
 * Speak text with the Kokoro voice (natural human speech, not the browser
 * voice). Returns when playback finishes. Any in-flight speech is stopped.
 */
export async function speakText(text: string): Promise<void> {
  const clean = text.replace(/```[\s\S]*?```/g, ' code block ').replace(/[#*_`>|]/g, '').trim();
  if (!clean) return;
  stopSpeech();
  const path = await invoke<string>('media_speak_text', { text: clean.slice(0, 5000) });
  await new Promise<void>((resolve, reject) => {
    const audio = new Audio(convertFileSrc(path));
    currentAudio = audio;
    audio.onended = () => {
      if (currentAudio === audio) currentAudio = null;
      resolve();
    };
    audio.onerror = () => reject(new Error('audio playback failed'));
    void audio.play().catch(reject);
  });
}

/**
 * Record from the microphone until `stop()` is called, then transcribe.
 * Returns a controller: call `stop()` to finish and get the transcript text.
 */
export async function startMicRecording(): Promise<{ stop: () => Promise<string> }> {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
  const mime = MediaRecorder.isTypeSupported('audio/webm') ? 'audio/webm' : '';
  const rec = new MediaRecorder(stream, mime ? { mimeType: mime } : undefined);
  const chunks: Blob[] = [];
  rec.ondataavailable = (e) => {
    if (e.data.size > 0) chunks.push(e.data);
  };
  const done = new Promise<string>((resolve, reject) => {
    rec.onstop = () => {
      stream.getTracks().forEach((t) => t.stop());
      const blob = new Blob(chunks, { type: rec.mimeType || 'audio/webm' });
      void (async () => {
        try {
          if (!blob.size) throw new Error('empty recording — no audio captured');
          // Local-first: a configured Whisper model transcribes fully offline.
          if (await localSttAvailable()) {
            const pcm = await blobToPcm16k(blob);
            resolve(await invoke<string>('media_transcribe_pcm', { data_b64: float32ToBase64(pcm) }));
            return;
          }
          const base64 = await new Promise<string>((res, rej) => {
            const reader = new FileReader();
            reader.onloadend = () => res((reader.result as string).split(',')[1] ?? '');
            reader.onerror = () => rej(new Error('failed to read recording'));
            reader.readAsDataURL(blob);
          });
          if (!base64) throw new Error('empty recording — no audio captured');
          resolve(
            await invoke<string>('media_transcribe_mic', {
              audio_base64: base64,
              content_type: blob.type || 'audio/webm',
            }),
          );
        } catch (e) {
          reject(e);
        }
      })();
    };
    rec.onerror = () => reject(new Error('recording failed'));
  });
  rec.start();
  return { stop: () => { rec.stop(); return done; } };
}
