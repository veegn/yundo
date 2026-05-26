import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { formatBytes, timeAgo, getIconForFileName } from '../utils/formatters';
import { useSeo } from '../utils/seo';
import { withBasePath } from '../utils/basePath';
import { useI18n } from '../context/I18nContext';

interface HistoryItem {
  slug: string;
  url: string;
  file_name: string;
  file_size: number;
  last_download_at: string;
  count_7d: number;
  score: number;
}

export default function Dashboard() {
  const [url, setUrl] = useState('');
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const { locale, t } = useI18n();

  useSeo({
    title: t('seo.dashboard.title'),
    description: t('seo.dashboard.description'),
    canonicalPath: '/',
    keywords: t('seo.dashboard.keywords'),
  });

  useEffect(() => {
    fetch(withBasePath('/api/recent'))
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then((data) => {
        setHistory(data.slice(0, 5));
        setLoading(false);
      })
      .catch((err) => {
        console.error('Failed to fetch history:', err);
        setLoading(false);
      });
  }, []);

  const handleDownload = () => {
    if (!url) return;
    window.open(`${withBasePath('/api/proxy')}?url=${encodeURIComponent(url)}`, '_blank');
  };

  const handleDownloadHistory = (historyUrl: string) => {
    window.open(`${withBasePath('/api/proxy')}?url=${encodeURIComponent(historyUrl)}`, '_blank');
  };

  return (
    <main className="flex-grow flex flex-col items-center px-6 pt-24 pb-16 max-w-7xl mx-auto w-full">
      <section className="w-full max-w-2xl text-center mb-24">
        <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-4">
          {t('dashboard.title')}
        </h1>
        <p className="text-on-surface-variant mb-12">
          {t('dashboard.subtitle')}
        </p>
        <div className="flex flex-col md:flex-row gap-3 p-2 bg-surface-container-lowest rounded-xl ghost-border ghost-shadow">
          <div className="flex-grow flex items-center px-4 bg-surface-container-lowest">
            <span className="material-symbols-outlined text-outline mr-3">link</span>
            <input
              className="w-full bg-transparent border-none focus:ring-0 text-on-surface placeholder:text-outline/60 py-3 font-medium outline-none"
              placeholder={t('dashboard.placeholder')}
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleDownload()}
            />
          </div>
          <button
            onClick={handleDownload}
            className="bg-primary-gradient text-on-primary px-8 py-3 rounded-lg font-semibold flex items-center justify-center gap-2 hover:opacity-90 active:scale-[0.98] transition-all cursor-pointer"
          >
            <span>{t('dashboard.btn_download')}</span>
            <span className="material-symbols-outlined text-sm">rocket_launch</span>
          </button>
        </div>
      </section>

      <section className="w-full max-w-4xl">
        <div className="flex justify-between items-end mb-6">
          <h2 className="text-xl font-bold text-on-surface">{t('dashboard.hot_downloads')}</h2>
        </div>

        <div className="bg-surface-container-low rounded-xl overflow-hidden">
          <div className="divide-y divide-outline-variant/10">
            {loading ? (
              <div className="p-8 text-center text-on-surface-variant">{t('dashboard.loading')}</div>
            ) : history.length === 0 ? (
              <div className="p-8 text-center text-on-surface-variant">{t('dashboard.no_history')}</div>
            ) : (
              history.map((item, index) => (
                <div
                  key={index}
                  className="flex items-center justify-between p-5 hover:bg-surface-container-high transition-colors bg-surface-container-lowest group cursor-pointer"
                  onClick={() => handleDownloadHistory(item.url)}
                >
                  <div className="flex items-center gap-4 overflow-hidden">
                    <div className="w-10 h-10 rounded bg-secondary-fixed flex items-center justify-center shrink-0 relative">
                      <span className="material-symbols-outlined text-on-secondary-fixed">
                        {getIconForFileName(item.file_name)}
                      </span>
                      {index === 0 && (
                        <span className="absolute -top-2 -right-2 text-lg" title={locale === 'zh' ? '最热门下载' : 'Most Popular'}>
                          🔥
                        </span>
                      )}
                    </div>
                    <div className="overflow-hidden">
                      <span
                        className="font-bold text-sm text-on-surface truncate block hover:text-primary transition-colors"
                        title={item.file_name}
                      >
                        {item.file_name}
                      </span>
                      <p
                        className="text-xs text-on-surface-variant font-mono truncate max-w-[200px] sm:max-w-xs md:max-w-md"
                        title={item.url}
                      >
                        {item.url}
                      </p>
                    </div>
                  </div>
                  <div className="flex items-center gap-4 shrink-0">
                    <div className="hidden sm:flex flex-col items-end">
                      <span className="text-xs font-medium text-on-surface-variant">
                        {formatBytes(item.file_size)}
                      </span>
                      <span className="text-[10px] text-on-surface-variant/70">
                        {t('dashboard.count_7d', { count: item.count_7d })}
                      </span>
                    </div>
                    <span className="text-xs text-on-surface-variant font-mono tabular-nums w-20 text-right">
                      {timeAgo(item.last_download_at, locale)}
                    </span>
                    <button
                      className="w-8 h-8 rounded-full bg-primary-container text-on-primary-container flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity cursor-pointer"
                      title={t('dashboard.re_download')}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDownloadHistory(item.url);
                      }}
                    >
                      <span className="material-symbols-outlined text-sm">download</span>
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </section>
    </main>
  );
}

