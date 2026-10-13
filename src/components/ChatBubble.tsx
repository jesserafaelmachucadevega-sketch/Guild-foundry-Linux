import React, { useState } from 'react';
import { copyToClipboard } from '../lib/api';
import type { Artifact, ChatMessage } from '../lib/types';
import { ArtifactRenderer } from './ArtifactRenderer';

interface Props {
  message: ChatMessage;
  onToast: (text: string) => void;
  onBranch?: (messageId: string) => void;
  onReply?: (message: ChatMessage) => void;
  onEmojiReaction?: (messageId: string, emoji: string) => void;
  reactions?: Record<string, Record<string, number>>;
}

const EMOJI_REACTIONS = ['😊','😂','❓','❗','🤯','😛'];

function renderText(content: string): React.ReactNode {
  const parts = content.split(/(```[\s\S]*?```)/g);
  return parts.map((part, i) => {
    if (part.startsWith('```')) {
      const code = part.replace(/^```[a-zA-Z]*\n?/, '').replace(/```$/, '');
      return <pre key={i}>{code}</pre>;
    }
    return <span key={i}>{part}</span>;
  });
}

export function ChatBubble({ message, onToast, onBranch, onReply, onEmojiReaction, reactions }: Props): React.ReactElement {
  const [showEmojis, setShowEmojis] = useState(false);
  const isUser = message.role === 'user';

  const copy = async (): Promise<void> => {
    try {
      await copyToClipboard(message.content);
      onToast('Copied to clipboard');
    } catch {
      onToast('Copy failed');
    }
  };

  const branch = (): void => {
    if (onBranch) onBranch(message.id);
  };

  const reply = (): void => {
    if (onReply) onReply(message);
  };

  const react = (emoji: string): void => {
    if (onEmojiReaction) onEmojiReaction(message.id, emoji);
    setShowEmojis(false);
  };

  const msgReactions = reactions?.[message.id];

  return (
    <div className={`gf-bubble gf-bubble-${isUser ? 'user' : 'assistant'}`}>
      <div className="gf-bubble-header">
        <span className="gf-bubble-role">
          {isUser ? 'You' : message.modelName ?? message.role}
        </span>
        {msgReactions && Object.keys(msgReactions).length > 0 && (
          <span className="gf-reaction-summary">
            {Object.entries(msgReactions)
              .map(([emoji, count]) => `${emoji} ${count}`)
              .join(' ')}
          </span>
        )}
      </div>
      <div className="gf-bubble-body">
        {renderText(message.content)}
        {(message.artifacts ?? []).map((a: Artifact, i: number) => (
          <ArtifactRenderer key={i} artifact={a} />
        ))}
      </div>
      <div className="gf-bubble-actions-row">
        {onReply && (
          <button className="gf-icon-btn gf-reply" onClick={reply} title="Reply">
            ↩
          </button>
        )}
        <button className="gf-icon-btn gf-emoji-btn" onClick={() => setShowEmojis((s) => !s)} title="React">
          😊
        </button>
        {showEmojis && (
          <div className="gf-emoji-picker">
            {EMOJI_REACTIONS.map((e) => (
              <button key={e} className="gf-emoji-option" onClick={() => react(e)}>
                {e}
              </button>
            ))}
          </div>
        )}
        {onBranch && message.role !== 'user' && (
          <button className="gf-icon-btn" onClick={branch} title="Fork thread from here">
            Fork
          </button>
        )}
        <button className="gf-icon-btn" onClick={copy} title="Copy to clipboard">
          Copy
        </button>
      </div>
    </div>
  );
}
