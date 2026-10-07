// SPDX-License-Identifier: Apache-2.0
// Agent-tab avatar: a still portrait by default; looping activity clips
// (typing / talking / waiting / idle) while the agent works. Agent tab ONLY —
// never used for builder or conference members.

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { appDataDir, join } from '@tauri-apps/api/path';

import girlDefault from '../../assets/avatar/girl-default.png';
import boyDefault from '../../assets/avatar/boy-default.png';

export type AvatarGender = 'girl' | 'boy';
export type AvatarActivity = 'idle' | 'typing' | 'talking' | 'waiting';

const DEFAULT_STILL: Record<AvatarGender, string> = {
  girl: girlDefault,
  boy: boyDefault,
};

export const AVATAR_GENDER_KEY = 'avatar.gender';

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
  return <img src={DEFAULT_STILL[gender]} style={style} title={title} alt="agent avatar" />;
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
