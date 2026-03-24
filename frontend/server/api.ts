import { Router } from 'express';
import https from 'https';
import http from 'http';
import { URL } from 'url';

const router = Router();

function sanitizeAsciiFilename(fileName: string) {
  const sanitized = fileName
    .replace(/["\\/:*?<>|]/g, '_')
    .replace(/[^\x20-\x7E]/g, '_')
    .trim()
    .replace(/^\.+|\.+$/g, '');

  return sanitized || 'download.bin';
}

function percentEncodeUtf8(value: string) {
  return Array.from(new TextEncoder().encode(value))
    .map((byte) => {
      const ch = String.fromCharCode(byte);
      return /[A-Za-z0-9\-._~]/.test(ch) ? ch : `%${byte.toString(16).toUpperCase().padStart(2, '0')}`;
    })
    .join('');
}

function buildContentDisposition(fileName: string) {
  const asciiName = sanitizeAsciiFilename(fileName);
  const encodedName = percentEncodeUtf8(fileName);
  return `attachment; filename="${asciiName}"; filename*=UTF-8''${encodedName}`;
}

// API Route: Proxy Download Endpoint
// This is a Node.js equivalent of the Rust proxy for preview purposes
router.get('/proxy', (req, res) => {
  const targetUrl = req.query.url as string;

  if (!targetUrl) {
    return res.status(400).json({ error: 'Missing url parameter' });
  }

  try {
    const parsedUrl = new URL(targetUrl);
    const host = parsedUrl.hostname;
    const fileName = parsedUrl.pathname.split('/').filter(Boolean).pop() || 'download.bin';

    // 1. Security Check: Prevent SSRF by blocking local/private IP ranges
    // In a real production app, you should use a more robust SSRF prevention library
    const isLocalOrPrivate = /^(localhost|127\.0\.0\.1|10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[0-1])\.)/.test(host);
    if (isLocalOrPrivate) {
      return res.status(403).json({ error: 'Access to local or private networks is forbidden.' });
    }

    // 2. Prepare the proxy request
    const client = parsedUrl.protocol === 'https:' ? https : http;
    
    const options = {
      headers: {
        'User-Agent': 'PrecisionLab-Proxy/1.0',
        ...(req.headers.range ? { 'Range': req.headers.range } : {})
      }
    };

    // 3. Execute request and stream response
    const proxyReq = client.get(targetUrl, options, (proxyRes) => {
      // Handle redirects (GitHub often redirects to a CDN)
      if (proxyRes.statusCode && proxyRes.statusCode >= 300 && proxyRes.statusCode < 400 && proxyRes.headers.location) {
         // Simple redirect following (in production, you'd want a more robust redirect handler)
         res.redirect(proxyRes.statusCode, proxyRes.headers.location);
         return;
      }

      res.status(proxyRes.statusCode || 500);

      // 4. Forward allowed headers
      const allowedHeaders = [
        'content-type', 
        'content-length', 
        'content-disposition', 
        'accept-ranges', 
        'content-range'
      ];

      for (const [key, value] of Object.entries(proxyRes.headers)) {
        if (allowedHeaders.includes(key.toLowerCase()) && value) {
          res.setHeader(key, value);
        }
      }

      if (!proxyRes.headers['content-disposition']) {
        res.setHeader('content-disposition', buildContentDisposition(fileName));
      }

      // 5. Stream the response directly to the client (Zero-copy concept)
      proxyRes.pipe(res);
    });

    proxyReq.on('error', (err) => {
      console.error('Proxy request error:', err);
      res.status(502).json({ error: 'Failed to reach target server' });
    });

  } catch (e) {
    res.status(400).json({ error: 'Invalid URL format' });
  }
});

// API Route: Recent Downloads Endpoint
router.get('/recent', (req, res) => {
  // Mock history data for the dashboard
  const mockHistory = [
    {
      url: 'https://github.com/user/repo/releases/download/v1.0.0/app-release.apk',
      file_name: 'app-release.apk',
      file_size: 10485760,
      last_download_at: new Date().toISOString(),
      count_7d: 42,
      score: 9.5
    },
    {
      url: 'https://example.com/large-dataset.zip',
      file_name: 'large-dataset.zip',
      file_size: 1073741824,
      last_download_at: new Date(Date.now() - 86400000).toISOString(),
      count_7d: 15,
      score: 7.2
    },
    {
      url: 'https://example.com/video-tutorial.mp4',
      file_name: 'video-tutorial.mp4',
      file_size: 256000000,
      last_download_at: new Date(Date.now() - 172800000).toISOString(),
      count_7d: 8,
      score: 5.4
    }
  ];
  res.json(mockHistory);
});

export default router;
