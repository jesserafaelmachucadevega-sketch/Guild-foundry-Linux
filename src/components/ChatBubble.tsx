import React from 'react';
import { copyToClipboard } from '../lib/api';
import type { Artifact, ChatMessage } from '../lib/types';
import { ArtifactRenderer } from './ArtifactRenderer';

interface Props {
  message: ChatMessage;
  onToast: (text: string) => void;
  onBranch?: (messageId: string) => void;
}

function renderText(content: string): React.ReactNode {
  // Minimal safe rendering: split out fenced code blocks, escape the rest.
  const parts = content.split(/(```[\s\S]*?```)/g);
  return parts.map((part, i) => {
    if (part.startsWith('```')) {
      const code = part.replace(/^```[a-zA-Z]*\n?/, '').replace(/```$/, '');
      return <pre key={i}>{code}</pre>;
    }
    return <span key={i}>{part}</span>;
  });
}

export function ChatBubble({ message, onToast, onBranch }: Props): React.ReactElement {
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

  return (
    <div className="gf-bubble">
      <div className="gf-bubble-header">
        <span className="gf-bubble-role">
          {message.role === 'user'
            ? 'You'
            : message.modelName ?? message.role}
        </span>
        <span className="gf-bubble-actions">
          {onBranch && message.role !== 'user' && (
            <button className="gf-icon-btn" onClick={branch} title="Fork thread from here">
              Fork
            </button>
          )}
          <button className="gf-icon-btn" onClick={copy} title="Copy to clipboard">
            Copy
          </button>
        </span>
      </div>
      <div className="gf-bubble-body">
        {renderText(message.content)}
        {(message.artifacts ?? []).map((a: Artifact, i: number) => (
          <ArtifactRenderer key={i} artifact={a} />
        ))}
      </div>
    </div>
  );
}
