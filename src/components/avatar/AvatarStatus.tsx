// SPDX-License-Identifier: Apache-2.0
// Agent-tab avatar: a still portrait by default; looping activity clips
// (typing / talking / waiting / idle) while the agent works. Agent tab ONLY —
// never used for builder or conference members.

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { appDataDir, join } from '@tauri-apps/api/path';

import { GIRL_DEFAULT } from '../../assets/avatar/girl_default';
import { BOY_DEFAULT } from '../../assets/avatar/boy_default';

export type AvatarGender = 'girl' | 'boy';
export type AvatarActivity = 'idle' | 'typing' | 'talking' | 'waiting';

const DEFAULT_STILL: Record<AvatarGender, string> = {
  girl: GIRL_DEFAULT,
  boy: BOY_DEFAULT,
};

export const AVATAR_GENDER_KEY = 'avatar.gender';
export const AVATAR_CUSTOM_KEY = 'avatar.custom';

/** Custom portrait (upload or model redesign) overrides the default. */
async function customPortrait(): Promise<string | null> {
  try {
    const flag = await invoke<string | null>('settings_get', { key: AVATAR_CUSTOM_KEY });
    if (flag !== '1') return null;
    const dir = await appDataDir();
    return convertFileSrc(await join(dir, 'avatars', 'custom.png'));
  } catch {
    return null;
  }
}

type Listener = () => void;
const listeners = new Set<Listener>();
/** Subscribe to avatar changes (upload, redesign, clear). */
export function onAvatarChange(fn: Listener): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}
export function emitAvatarChange(): void {
  listeners.forEach((f) => f());
}

/** Activity clips live in app-data/avatars (downloaded, not bundled). */
async function clipPath(gender: AvatarGender, activity: AvatarActivity): Promise<string | null> {
  try {
    const dir = await appDataDir();
    const p = await join(dir, 'avatars', `${gender}-${activity}.webm`);
    return convertFileSrc(p);
  } catch {
    return null;
  }
}

interface Props {
  gender: AvatarGender;
  activity: AvatarActivity;
  size?: number;
  title?: string;
}

/**
 * The agent's face on the Agent tab. Still portrait by default; when the
 * agent is doing something and a clip exists for that activity, the clip
 * loops. Missing clips fall back to the still — never a broken frame.
 */
export function AvatarStatus({ gender, activity, size = 96, title }: Props): React.ReactElement {
  const [clip, setClip] = useState<string | null>(null);
  const [clipOk, setClipOk] = useState(true);
  const [still, setStill] = useState<string>(DEFAULT_STILL[gender]);

  useEffect(() => {
    let live = true;
    const read = () => {
      void customPortrait().then((c) => {
        if (live) setStill(c ?? DEFAULT_STILL[gender]);
      });
    };
    read();
    const off = onAvatarChange(read);
    return () => {
      live = false;
      off();
    };
  }, [gender]);

  useEffect(() => {
    let live = true;
    setClipOk(true);
    if (activity === 'idle') {
      setClip(null);
      return;
    }
    void clipPath(gender, activity).then((p) => {
      if (live) setClip(p);
    });
    return () => {
      live = false;
    };
  }, [gender, activity]);

  const style: React.CSSProperties = {
    width: size,
    height: size,
    borderRadius: '50%',
    objectFit: 'cover',
    // Feathered circular edge even for square sources.
    maskImage: 'radial-gradient(circle, black 78%, transparent 98%)',
    WebkitMaskImage: 'radial-gradient(circle, black 78%, transparent 98%)',
    background: 'transparent',
  };

  if (clip && clipOk) {
    return (
      <video
        src={clip}
        style={style}
        title={title}
        autoPlay
        loop
        muted
        playsInline
        onError={() => setClipOk(false)}
      />
    );
  }
  return <img src={still} style={style} title={title} alt="agent avatar" />;
}

/** Read the user's avatar choice (defaults to girl). */
export async function getAvatarGender(): Promise<AvatarGender> {
  try {
    const v = await invoke<string | null>('settings_get', { key: AVATAR_GENDER_KEY });
    return v === 'boy' ? 'boy' : 'girl';
  } catch {
    return 'girl';
  }
}
