import { useState, useEffect } from 'react';
import { formatBytes, formatDate, getIconForFileName } from '../utils/formatters';
import { useSeo } from '../utils/seo';
import { Link } from 'react-router-dom';
import type { HistoryItem } from '../types/api';

export default function ProxyDash() {
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(true);

  useSeo({
    title: '下载历史记录 - 云渡',
    description:
      '查看云渡最近处理过的热门下载记录，包括文件名、文件大小、最近处理时间和下载热度量。',
    canonicalPath: '/proxydash',
    keywords: '下载历史,热门下载,文件记录,下载热度',
  });

  useEffect(() => {
    fetch('/api/recent')
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then((data) => {
        setHistory(data);
        setLoading(false);
      })
      .catch((err) => {
        console.error('Failed to fetch history:', err);
        setLoading(false);
      });
  }, []);

  const handleDownload = (url: string) => {
    const proxyUrl = `/api/proxy?url=${encodeURIComponent(url)}`;
    window.open(proxyUrl, '_blank');
  };

  return (
    <main className="flex-grow max-w-7xl w-full mx-auto px-6 py-16 animate-in fade-in duration-700">
      <div className="mb-12 flex flex-col md:flex-row md:items-end justify-between gap-6">
        <div>
          <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-3 font-display">下载历史记录</h1>
          <p className="text-on-surface-variant text-lg max-w-2xl leading-relaxed">
            管理并重新下载公开处理过的文件记录。列表已按下载热度算法智能排序。
          </p>
        </div>
        <div className="flex items-center gap-4">
          <a 
            href="/downloads" 
            className="px-6 py-2.5 rounded-xl border border-outline-variant hover:bg-surface-container-high transition-all text-sm font-bold text-on-surface"
          >
            浏览公共资源
          </a>
        </div>
      </div>

      <div className="grid gap-6">
        {loading ? (
          <div className="col-span-full p-32 glass rounded-3xl text-center flex flex-col items-center gap-6">
            <div className="w-16 h-16 border-4 border-primary/20 border-t-primary rounded-full animate-spin" />
            <p className="text-on-surface-variant text-lg font-medium">深度同步历史数据中...</p>
          </div>
        ) : history.length === 0 ? (
          <div className="col-span-full p-32 glass rounded-3xl text-center text-on-surface-variant font-medium border-dashed border-2 border-outline-variant/30 text-xl">
            暂无处理记录
          </div>
        ) : (
          history.map((item, index) => (
            <div 
              key={index} 
              className="group glass rounded-2xl p-6 card-hover border-transparent hover:border-primary/20 transition-all flex flex-col lg:flex-row gap-8 items-start lg:items-center relative"
            >
              {index === 0 && (
                <div className="absolute -top-3 -left-3 px-3 py-1 bg-gradient-to-r from-[#FF6B00] to-[#FF9E00] text-white text-[10px] font-black uppercase tracking-tighter rounded-lg shadow-lg z-10 animate-bounce">
                  Best of All Time
                </div>
              )}

              <div className="flex items-center gap-6 flex-grow min-w-0">
                <div className="w-14 h-14 lg:w-20 lg:h-20 rounded-2xl bg-secondary-fixed flex items-center justify-center shrink-0 shadow-sm group-hover:rotate-3 transition-transform duration-500">
                  <span className="material-symbols-outlined text-3xl lg:text-4xl text-on-secondary-fixed">
                    {getIconForFileName(item.file_name)}
                  </span>
                </div>
                <div className="min-w-0 flex-grow">
                  <div className="flex items-center gap-3 mb-2">
                    <a
                      href={`/downloads/${item.slug}`}
                      className="text-xl font-bold text-on-surface truncate block hover:text-primary transition-colors"
                      title={item.file_name}
                    >
                      {item.file_name}
                    </a>
                  </div>
                  <div className="flex flex-wrap items-center gap-y-2 gap-x-6 text-sm font-medium text-on-surface-variant/70 font-mono">
                    <span className="flex items-center gap-1.5"><span className="material-symbols-outlined text-base">database</span> {formatBytes(item.file_size)}</span>
                    <span className="flex items-center gap-1.5"><span className="material-symbols-outlined text-base">schedule</span> {formatDate(item.last_download_at)}</span>
                    <span className="flex items-center gap-1.5 text-secondary"><span className="material-symbols-outlined text-base">star</span> 热度 {item.score.toFixed(1)}</span>
                  </div>
                </div>
              </div>

              <div className="w-full lg:w-auto flex items-center justify-between lg:justify-end gap-6 pt-6 lg:pt-0 border-t lg:border-t-0 border-outline-variant/10 shrink-0">
                <div className="flex flex-col items-center lg:items-end">
                  <span className="text-xl font-black text-on-surface tabular-nums">{item.count_7d}</span>
                  <span className="text-[10px] font-bold text-on-surface-variant uppercase tracking-widest">Downloads (7d)</span>
                </div>
                
                <div className="flex items-center gap-3">
                  <a
                    href={`/downloads/${item.slug}`}
                    className="px-6 py-3 text-sm font-bold text-secondary hover:bg-secondary/5 rounded-xl transition-colors"
                  >
                    详情
                  </a>
                  <button
                    onClick={() => handleDownload(item.url)}
                    className="flex items-center gap-3 px-8 py-3 bg-primary-gradient text-on-primary font-bold rounded-xl hover:shadow-lg hover:shadow-primary/25 active:scale-95 transition-all"
                  >
                    <span className="material-symbols-outlined text-xl">download_for_offline</span>
                    <span>立即下载</span>
                  </button>
                </div>
              </div>

              {/* URL Display for larger screens */}
              <div className="hidden lg:block absolute bottom-4 left-32 max-w-xl opacity-0 group-hover:opacity-100 transition-opacity">
                <p className="text-[10px] text-on-surface-variant font-mono truncate bg-surface-container-high px-2 py-1 rounded" title={item.url}>
                  {item.url}
                </p>
              </div>
            </div>
          ))
        )}
      </div>

      {!loading && history.length > 0 && (
        <div className="mt-16 p-8 glass rounded-2xl flex flex-col md:flex-row items-center justify-between gap-6 border-outline-variant/10">
          <p className="text-sm font-medium text-on-surface-variant">
            已同步 <span className="text-on-surface px-2 py-0.5 bg-surface-container rounded-md tabular-nums">{history.length}</span> 条活跃加速记录
          </p>
          <div className="flex items-center gap-2">
            <button className="px-4 py-2 text-sm font-bold opacity-50 cursor-not-allowed">上一页</button>
            <div className="flex items-center gap-1">
              <span className="px-4 py-2 bg-primary text-on-primary rounded-lg text-sm font-bold shadow-sm">1</span>
            </div>
            <button className="px-4 py-2 text-sm font-bold opacity-50 cursor-not-allowed">下一页</button>
          </div>
        </div>
      )}
    </main>
  );
}

