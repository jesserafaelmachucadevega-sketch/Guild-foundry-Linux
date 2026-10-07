// SPDX-License-Identifier: Apache-2.0
// Avatar picker for the Agent tab: choose the girl or the boy.
// The still is the default face; activity clips animate from it.

import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { GIRL_DEFAULT } from '../../assets/avatar/girl_default';
import { BOY_DEFAULT } from '../../assets/avatar/boy_default';
import { AVATAR_GENDER_KEY, emitAvatarChange, type AvatarGender } from './AvatarStatus';

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
  const [custom, setCustom] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void invoke<string | null>('settings_get', { key: AVATAR_GENDER_KEY })
      .then((v) => {
        if (v === 'boy' || v === 'girl') setGender(v);
      })
      .catch(() => {});
    void invoke<string | null>('settings_get', { key: 'avatar.custom' })
      .then((v) => setCustom(v === '1'))
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
    gender === g && !custom ? { outline: '2px solid var(--gf-gold)', outlineOffset: 3 } : {};

  const onFile = async (file: File | undefined) => {
    if (!file) return;
    if (file.size > 10 * 1024 * 1024) {
      onToast('Image must be under 10 MB');
      return;
    }
    try {
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const r = new FileReader();
        r.onload = () => resolve(r.result as string);
        r.onerror = () => reject(new Error('read failed'));
        r.readAsDataURL(file);
      });
      await invoke('avatar_upload_image', { dataBase64: dataUrl });
      setCustom(true);
      emitAvatarChange();
      onToast('Custom avatar set');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  const clearCustom = async () => {
    try {
      await invoke('avatar_clear_custom');
      setCustom(false);
      emitAvatarChange();
      onToast('Back to the default avatar');
    } catch (e) {
      onToast(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div>
      <span className="gf-muted conn-small">Agent avatar</span>
      <div className="gf-row" style={{ gap: 12, marginTop: 6 }}>
        <img
          src={GIRL_DEFAULT}
          style={{ ...circle, ...ring('girl') }}
          onClick={() => void pick('girl')}
          title="Her"
          alt="girl avatar"
        />
        <img
          src={BOY_DEFAULT}
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
      <div className="gf-row" style={{ gap: 8, marginTop: 8, flexWrap: 'wrap' }}>
        <input
          ref={fileRef}
          type="file"
          accept="image/png,image/jpeg,image/webp"
          style={{ display: 'none' }}
          onChange={(e) => {
            void onFile(e.target.files?.[0]);
            e.target.value = '';
          }}
        />
        <button className="gf-icon-btn" onClick={() => fileRef.current?.click()}>
          Upload picture
        </button>
        {custom && (
          <button className="gf-icon-btn" onClick={() => void clearCustom()}>
            Use default
          </button>
        )}
      </div>
      <p className="conn-note" style={{ marginTop: 8 }}>
        Don't like these? Upload your own picture — or ask the agent to
        redesign it ("make my avatar a cyberpunk robot") and it will generate
        and set it for you.
      </p>
    </div>
  );
}
