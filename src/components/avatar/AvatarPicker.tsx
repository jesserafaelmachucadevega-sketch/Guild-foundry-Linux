// SPDX-License-Identifier: Apache-2.0
// Avatar picker for the Agent tab: choose the girl or the boy.
// The still is the default face; activity clips animate from it.

import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import girlDefault from '../../assets/avatar/girl-default.png';
import boyDefault from '../../assets/avatar/boy-default.png';
import { AVATAR_GENDER_KEY, type AvatarGender } from './AvatarStatus';

const circle: React.CSSProperties = {
  width: 72,
  height: 72,
  borderRadius: '50%',
  objectFit: 'cover',
  cursor: 'pointer',
  maskImage: 'radial-gradient(circle, black 78%, transparent 98%)',
  WebkitMaskImage: 'radial-gradient(circle, black 78%, transparent 98%)',
};

export function AvatarPicker({ onToast }: { onToast: (t: string) => void }): React.ReactElement {
  const [gender, setGender] = useState<AvatarGender>('girl');

  useEffect(() => {
    void invoke<string | null>('settings_get', { key: AVATAR_GENDER_KEY })
      .then((v) => {
        if (v === 'boy' || v === 'girl') setGender(v);
      })
      .catch(() => {});
  }, []);

  const pick = async (g: AvatarGender) => {
    setGender(g);
    try {
      await invoke('settings_set', { key: AVATAR_GENDER_KEY, value: g });
      onToast(g === 'girl' ? 'Avatar: her' : 'Avatar: him');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const ring = (g: AvatarGender): React.CSSProperties =>
    gender === g ? { outline: '2px solid var(--gf-gold)', outlineOffset: 3 } : {};

  return (
    <div>
      <span className="gf-muted conn-small">Agent avatar</span>
      <div className="gf-row" style={{ gap: 12, marginTop: 6 }}>
        <img
          src={girlDefault}
          style={{ ...circle, ...ring('girl') }}
          onClick={() => void pick('girl')}
          title="Her"
          alt="girl avatar"
        />
        <img
          src={boyDefault}
          style={{ ...circle, ...ring('boy') }}
          onClick={() => void pick('boy')}
          title="Him"
          alt="boy avatar"
        />
      </div>
      <p className="conn-note" style={{ marginTop: 8 }}>
        The still is the face you see by default. Animated clips in
        app-data/avatars (girl-typing.webm, boy-talking.webm, …) play for
        each activity when present.
      </p>
    </div>
  );
}
