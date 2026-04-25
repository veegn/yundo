declare global {
  interface Window {
    __YUNDO_BASE_PATH__?: string;
  }
}

export function getBasePath() {
  const raw = window.__YUNDO_BASE_PATH__ ?? '/';
  if (!raw || raw === '/') return '/';
  return raw.endsWith('/') ? raw.slice(0, -1) : raw;
}

export function withBasePath(path: string) {
  if (/^https?:\/\//.test(path)) return path;

  const basePath = getBasePath();
  const normalizedPath = path.startsWith('/') ? path : `/${path}`;
  return basePath === '/' ? normalizedPath : `${basePath}${normalizedPath}`;
}
