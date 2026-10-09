import { invoke } from '@tauri-apps/api/core';
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
      const reader = new FileReader();
      reader.onloadend = async () => {
        try {
          const base64 = (reader.result as string).split(',')[1] ?? '';
          const text = await invoke<string>('media_transcribe_mic', {
            audioBase64: base64,
            contentType: blob.type || 'audio/webm',
          });
          resolve(text);
        } catch (e) {
          reject(e);
        }
      };
      reader.onerror = () => reject(new Error('failed to read recording'));
      reader.readAsDataURL(blob);
    };
    rec.onerror = () => reject(new Error('recording failed'));
  });
  rec.start();
  return { stop: () => { rec.stop(); return done; } };
}
