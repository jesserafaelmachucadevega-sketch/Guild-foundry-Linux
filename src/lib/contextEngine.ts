// SPDX-License-Identifier: Apache-2.0
// Phase 6 — context engine (master spec section 49): relevance ranking,
// token budgeting, compression, file summaries, retrieval, caching.
//
// Retrieval here is keyword-overlap similarity — a cheap stand-in labeled as
// such. True embedding-based semantic retrieval is Phase 10's job.

/** Rough token estimate: ~4 characters per token (spec-mandated heuristic). */
export const CHARS_PER_TOKEN = 4;

export function estimateTokens(text: string): number {
  return Math.ceil(text.length / CHARS_PER_TOKEN);
}

function tokenize(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^a-z0-9_]+/)
    .filter((t) => t.length > 2);
}

export interface RankedItem<T> {
  item: T;
  /** 0..1 — fraction of distinct query terms found in the item. */
  score: number;
}

/**
 * Relevance ranking: score each item by keyword overlap with the query,
 * highest first. Deterministic; ties keep original order.
 */
export function rankByRelevance<T>(
  items: T[],
  query: string,
  textOf: (item: T) => string,
): Array<RankedItem<T>> {
  const qTerms = new Set(tokenize(query));
  if (qTerms.size === 0) return items.map((item) => ({ item, score: 0 }));
  return items
    .map((item) => {
      const termSet = new Set(tokenize(textOf(item)));
      let overlap = 0;
      for (const q of qTerms) {
        if (termSet.has(q)) overlap += 1;
      }
      return { item, score: overlap / qTerms.size };
    })
    .sort((a, b) => b.score - a.score);
}

export interface BudgetFit<T> {
  kept: T[];
  /** How many items were dropped because they did not fit the budget. */
  omitted: number;
  usedTokens: number;
}

/**
 * Token budgeting: keep the most relevant items that fit inside
 * `budgetTokens`. The caller should surface `omitted` to the user (spec:
 * "display when important context was omitted due to limits").
 */
export function fitToBudget<T>(
  items: T[],
  budgetTokens: number,
  query: string,
  textOf: (item: T) => string,
): BudgetFit<T> {
  const ranked = rankByRelevance(items, query, textOf);
  const kept: T[] = [];
  let used = 0;
  let omitted = 0;
  for (const { item } of ranked) {
    const cost = estimateTokens(textOf(item));
    if (used + cost <= budgetTokens) {
      kept.push(item);
      used += cost;
    } else {
      omitted += 1;
    }
  }
  return { kept, omitted, usedTokens: used };
}

/**
 * Compression pass: collapse horizontal whitespace and runs of blank lines.
 * Lossy but structure-preserving — use before `truncatePreserve`.
 */
export function compressText(text: string): string {
  return text
    .replace(/[ \t]+/g, ' ')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/**
 * Truncate to `maxTokens`, preserving the head AND the tail (where
 * conclusions, signatures, and exports usually live) with an explicit
 * omission marker between them.
 */
export function truncatePreserve(text: string, maxTokens: number): string {
  const maxChars = maxTokens * CHARS_PER_TOKEN;
  if (text.length <= maxChars) return text;
  const marker = '\n…[omitted: content truncated to fit the token budget]…\n';
  const headChars = Math.floor((maxChars - marker.length) * 0.6);
  const tailChars = maxChars - marker.length - headChars;
  if (headChars <= 0 || tailChars <= 0) return marker.trim();
  return (
    text.slice(0, headChars) + marker + text.slice(Math.max(0, text.length - tailChars))
  );
}

export interface FileSummary {
  path: string;
  /** First `maxLines` lines of the file. */
  headLines: string[];
  /** Symbols found by the scanner (functions, classes, exports, structs, ...). */
  symbols: string[];
  lineCount: number;
  truncated: boolean;
}

/**
 * File summarization: first-N-lines + symbol scan. Keeps the model's view of
 * a file compact without sending the whole file blindly.
 */
export function summarizeFile(path: string, content: string, maxLines = 40): FileSummary {
  const lines = content.split('\n');
  const patterns: RegExp[] = [
    /^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)/,
    /^\s*(?:export\s+)?(?:default\s+)?class\s+([A-Za-z_$][\w$]*)/,
    /^\s*export\s+(?:const|let|var)\s+([A-Za-z_$][\w$]*)/,
    /^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)/,
    /^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)\s*=/,
    /^\s*(?:pub\s+)?fn\s+([A-Za-z_$][\w$]*)/,
    /^\s*(?:pub\s+)?struct\s+([A-Za-z_$][\w$]*)/,
    /^\s*(?:pub\s+)?enum\s+([A-Za-z_$][\w$]*)/,
    /^\s*def\s+([A-Za-z_$][\w$]*)/,
  ];
  const symbols: string[] = [];
  for (const line of lines) {
    for (const p of patterns) {
      const m = p.exec(line);
      if (m && !symbols.includes(m[1])) symbols.push(m[1]);
      if (symbols.length >= 60) break;
    }
    if (symbols.length >= 60) break;
  }
  return {
    path,
    headLines: lines.slice(0, maxLines),
    symbols,
    lineCount: lines.length,
    truncated: lines.length > maxLines,
  };
}

/** Format a FileSummary as compact model-readable text. */
export function formatFileSummary(s: FileSummary): string {
  const head = [`FILE: ${s.path} (${s.lineCount} lines${s.truncated ? ', truncated' : ''})`];
  if (s.symbols.length > 0) head.push(`SYMBOLS: ${s.symbols.join(', ')}`);
  head.push('HEAD:');
  head.push(s.headLines.join('\n'));
  return head.join('\n');
}

/**
 * Tiny recency-based cache for context-engine outputs (summaries, ranked
 * lists). Keyed by the caller; no eviction policy beyond a size cap.
 */
export class ContextCache {
  private map = new Map<string, string>();

  constructor(private maxEntries = 200) {}

  get(key: string): string | undefined {
    const v = this.map.get(key);
    if (v === undefined) return undefined;
    // Refresh recency.
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }

  set(key: string, value: string): void {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    while (this.map.size > this.maxEntries) {
      const oldest = this.map.keys().next();
      if (oldest.done) break;
      this.map.delete(oldest.value);
    }
  }

  clear(): void {
    this.map.clear();
  }

  get size(): number {
    return this.map.size;
  }
}
