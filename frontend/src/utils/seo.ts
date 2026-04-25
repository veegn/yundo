import { useEffect } from 'react';
import { withBasePath } from './basePath';

type SeoOptions = {
  title: string;
  description: string;
  canonicalPath?: string;
  robots?: string;
  keywords?: string;
};

const DEFAULT_SITE_NAME = '云渡';

export function useSeo({
  title,
  description,
  canonicalPath,
  robots = 'index,follow',
  keywords,
}: SeoOptions) {
  useEffect(() => {
    const canonicalUrl = canonicalPath
      ? `${window.location.origin}${withBasePath(canonicalPath)}`
      : `${window.location.origin}${window.location.pathname}${window.location.search}`;
    document.title = title;

    setMeta('name', 'description', description);
    setMeta('name', 'robots', robots);
    setMeta('property', 'og:type', 'website');
    setMeta('property', 'og:site_name', DEFAULT_SITE_NAME);
    setMeta('property', 'og:title', title);
    setMeta('property', 'og:description', description);
    setMeta('property', 'og:url', canonicalUrl);
    setMeta('name', 'twitter:card', 'summary_large_image');
    setMeta('name', 'twitter:title', title);
    setMeta('name', 'twitter:description', description);
    if (keywords) {
      setMeta('name', 'keywords', keywords);
    }

    setCanonical(canonicalUrl);
  }, [canonicalPath, description, keywords, robots, title]);
}

function setMeta(attribute: 'name' | 'property', key: string, content: string) {
  let meta = document.head.querySelector<HTMLMetaElement>(`meta[${attribute}="${key}"]`);
  if (!meta) {
    meta = document.createElement('meta');
    meta.setAttribute(attribute, key);
    document.head.appendChild(meta);
  }
  meta.setAttribute('content', content);
}

function setCanonical(href: string) {
  let link = document.head.querySelector<HTMLLinkElement>('link[rel="canonical"]');
  if (!link) {
    link = document.createElement('link');
    link.setAttribute('rel', 'canonical');
    document.head.appendChild(link);
  }
  link.setAttribute('href', href);
}
