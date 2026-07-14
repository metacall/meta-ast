export function formatHashResult(digest: string, algo: string): string {
  return `${algo}:${digest}`;
}

export const DYNAMIC = 1;
