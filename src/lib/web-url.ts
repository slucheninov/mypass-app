const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const HOSTLIKE = /^[^\s/?#:@]+(\.[^\s/?#:@]+)+([/?#]\S*)?$/;

/**
 * The page a stored website opens, or `null` when it is not a web page. Only `http` and `https`
 * pass (R32); a scheme-less value is read as `https`, exactly like the entry matcher does, but
 * only when it looks like a host name. Everything else (`javascript:`, `file:`, ...) is plain text.
 */
export function webUrl(raw: string): string | null {
  const text = raw.trim();
  if (text === "") return null;
  const candidate = HAS_SCHEME.test(text) ? text : HOSTLIKE.test(text) ? `https://${text}` : null;
  if (candidate === null) return null;
  try {
    const url = new URL(candidate);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}
