import React, { useEffect, useRef, useState } from 'react';
import { getSpeechEnabled, setSpeechEnabled, startMicRecording, stopSpeech } from './speech';

interface Props {
  /** Transcript lands here — the parent puts it in its input or sends it. */
  onTranscript: (text: string) => void;
  onToast: (msg: string) => void;
  disabled?: boolean;
}

/**
 * Mic button (record → transcribe → text) + speech mute/unmute toggle.
 * When speech is unmuted, the parent should speak new agent messages
 * immediately (see speakText).
 */
export function VoiceControls({ onTranscript, onToast, disabled }: Props) {
  const [speechOn, setSpeechOn] = useState(false);
  const [recording, setRecording] = useState(false);
  const [working, setWorking] = useState(false);
  const recRef = useRef<{ stop: () => Promise<string> } | null>(null);

  useEffect(() => {
    void getSpeechEnabled().then(setSpeechOn);
    return () => stopSpeech();
  }, []);

  const toggleSpeech = async () => {
    const next = !speechOn;
    setSpeechOn(next);
    await setSpeechEnabled(next);
    onToast(next ? 'Speech on — agent replies will be read aloud' : 'Speech off');
  };

  const toggleMic = async () => {
    if (recording) {
      // Stop and transcribe.
      setWorking(true);
      try {
        const text = await recRef.current!.stop();
        recRef.current = null;
        setRecording(false);
        if (text.trim()) onTranscript(text.trim());
        else onToast('Did not catch that — try again');
      } catch (e) {
        onToast(`Mic failed: ${e instanceof Error ? e.message : String(e)}`);
      } finally {
        setWorking(false);
      }
      return;
    }
    try {
      recRef.current = await startMicRecording();
      setRecording(true);
    } catch {
      onToast('Microphone unavailable — check browser permission');
    }
  };

  return (
    <span className="gf-row" style={{ gap: 4 }}>
      <button
        className={`gf-icon-btn${recording ? ' gf-rec' : ''}`}
        onClick={() => void toggleMic()}
        disabled={disabled || working}
        title={recording ? 'Stop and transcribe' : 'Dictate'}
      >
        {working ? '…' : recording ? 'Stop' : 'Mic'}
      </button>
      <button
        className="gf-icon-btn"
        onClick={() => void toggleSpeech()}
        disabled={disabled}
        title={speechOn ? 'Mute speech' : 'Unmute speech'}
      >
        {speechOn ? 'Sound on' : 'Muted'}
      </button>
    </span>
  );
}
