import { Link } from 'react-router-dom';
import { useState, useEffect } from 'react';
import { formatBytes, timeAgo, getIconForFileName } from '../utils/formatters';
import { useSeo } from '../utils/seo';
import type { HistoryItem } from '../types/api';

export default function Dashboard() {
  const [url, setUrl] = useState('');
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(true);

  useSeo({
    title: '云渡 - HTTP/HTTPS 下载代理与文件加速',
    description:
      '云渡提供稳定的 HTTP/HTTPS 文件下载代理、断点续传支持和热门下载记录展示，帮助用户更高效地获取公开下载资源。',
    canonicalPath: '/',
    keywords: '下载代理,HTTP下载,HTTPS下载,文件加速,断点续传,下载历史',
  });

  useEffect(() => {
    fetch('/api/recent')
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then((data) => {
        setHistory(data.slice(0, 3));
        setLoading(false);
      })
      .catch((err) => {
        console.error('Failed to fetch history:', err);
        setLoading(false);
      });
  }, []);

  const handleDownload = () => {
    if (!url) return;
    const proxyUrl = `/api/proxy?url=${encodeURIComponent(url)}`;
    window.open(proxyUrl, '_blank');
  };

  const handleDownloadHistory = (historyUrl: string) => {
    const proxyUrl = `/api/proxy?url=${encodeURIComponent(historyUrl)}`;
    window.open(proxyUrl, '_blank');
  };

  return (
    <main className="flex-grow flex flex-col items-center px-6 pt-32 pb-24 max-w-7xl mx-auto w-full relative overflow-hidden">
      {/* Background Decorative Elements */}
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-full h-[600px] -z-10 bg-gradient-to-b from-primary/5 to-transparent rounded-[100%] blur-3xl opacity-50" />
      
      <section className="w-full max-w-3xl text-center mb-32 relative animate-in fade-in slide-in-from-bottom-8 duration-1000">
        <div className="inline-flex items-center gap-2 px-4 py-1.5 rounded-full bg-primary/10 border border-primary/20 text-primary text-xs font-bold mb-8 uppercase tracking-widest animate-pulse-slow">
          <span className="relative flex h-2 w-2">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-primary opacity-75"></span>
            <span className="relative inline-flex rounded-full h-2 w-2 bg-primary"></span>
          </span>
          Next Gen Download Proxy
        </div>
        
        <h1 className="text-5xl md:text-7xl font-extrabold tracking-tight mb-6 font-display">
          <span className="text-on-surface">极简</span>
          <span className="text-gradient">代理</span>
          <span className="text-on-surface"> 极速</span>
          <span className="text-gradient">下载</span>
        </h1>
        
        <p className="text-lg text-on-surface-variant max-w-xl mx-auto mb-12 leading-relaxed">
          输入任意链接，即刻体验无阻断、全速的文件获取。
          支持大型文件，多线程加速，让下载飞起来。
        </p>

        <div className="flex flex-col md:flex-row gap-3 p-3 glass rounded-2xl ghost-shadow relative group">
          <div className="absolute -inset-1 bg-gradient-to-r from-primary to-secondary rounded-2xl opacity-0 group-focus-within:opacity-10 transition-opacity blur-lg" />
          <div className="flex-grow flex items-center px-4 bg-surface-container-lowest/50 rounded-xl relative">
            <span className="material-symbols-outlined text-outline group-focus-within:text-primary transition-colors">link</span>
            <input
              className="w-full bg-transparent border-none focus:ring-0 text-on-surface placeholder:text-outline/50 py-4 px-3 font-medium outline-none text-lg"
              placeholder="粘贴下载链接..."
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleDownload()}
            />
          </div>
          <button
            onClick={handleDownload}
            className="bg-primary-gradient text-on-primary px-10 py-4 rounded-xl font-bold flex items-center justify-center gap-3 hover:shadow-lg hover:shadow-primary/25 active:scale-[0.98] transition-all relative overflow-hidden group/btn"
          >
            <span className="relative z-10 text-lg">开始下载</span>
            <span className="material-symbols-outlined text-xl group-hover/btn:translate-x-1 transition-transform relative z-10">rocket_launch</span>
          </button>
        </div>
        
        <div className="mt-8 flex items-center justify-center gap-6 text-sm text-on-surface-variant/70 font-medium">
          <div className="flex items-center gap-1.5"><span className="material-symbols-outlined text-lg">bolt</span> 高速通道</div>
          <div className="flex items-center gap-1.5"><span className="material-symbols-outlined text-lg">shield_check</span> 安全匿名</div>
          <div className="flex items-center gap-1.5"><span className="material-symbols-outlined text-lg">cloud_sync</span> 断点续传</div>
        </div>
      </section>

      <section className="w-full max-w-5xl animate-in fade-in slide-in-from-bottom-12 duration-1000 delay-300">
        <div className="flex justify-between items-end mb-8 px-2">
          <div>
            <h2 className="text-2xl font-bold text-on-surface flex items-center gap-2">
              热门下载 
              <span className="px-2 py-0.5 rounded bg-tertiary-fixed text-on-tertiary text-[10px] font-black uppercase tracking-tighter">🔥 Hot</span>
            </h2>
            <p className="text-sm text-on-surface-variant">社区用户最近加速获取的高频资源</p>
          </div>
          <div className="flex items-center gap-6">
            <Link
              to="/proxydash"
              className="group text-sm font-bold text-secondary flex items-center gap-1 transition-all"
            >
              <span>查看全部历史</span>
              <span className="material-symbols-outlined text-sm group-hover:translate-x-1 transition-transform">arrow_forward</span>
            </Link>
          </div>
        </div>

        <div className="grid gap-4">
          {loading ? (
            <div className="p-20 glass rounded-3xl text-center flex flex-col items-center gap-4">
              <div className="w-12 h-12 border-4 border-primary/20 border-t-primary rounded-full animate-spin" />
              <p className="text-on-surface-variant font-medium">正在获取最新数据...</p>
            </div>
          ) : history.length === 0 ? (
            <div className="p-20 glass rounded-3xl text-center text-on-surface-variant font-medium border-dashed border-2 border-outline-variant/30 text-lg">
              暂无下载记录，快来开启第一次下载吧
            </div>
          ) : (
            history.map((item, index) => (
              <div
                key={index}
                className="group relative glass rounded-2xl p-6 card-hover cursor-pointer border-transparent hover:border-primary/20 transition-all"
                onClick={() => handleDownloadHistory(item.url)}
              >
                <div className="flex items-center gap-6">
                  <div className="w-16 h-16 rounded-2xl bg-secondary-fixed flex items-center justify-center shrink-0 relative shadow-sm group-hover:scale-110 transition-transform duration-500">
                    <span className="material-symbols-outlined text-3xl text-on-secondary-fixed">
                      {getIconForFileName(item.file_name)}
                    </span>
                    {index === 0 && (
                      <div className="absolute -top-3 -right-3 w-8 h-8 bg-[#FF6B00] rounded-full flex items-center justify-center text-white text-lg shadow-lg border-2 border-surface animate-bounce">
                        🔥
                      </div>
                    )}
                  </div>
                  
                  <div className="flex-grow overflow-hidden">
                    <div className="flex items-center gap-3 mb-1">
                      <a
                        href={`/downloads/${item.slug}`}
                        className="font-bold text-lg text-on-surface truncate block hover:text-primary transition-colors"
                        title={item.file_name}
                        onClick={(e) => e.stopPropagation()}
                      >
                        {item.file_name}
                      </a>
                    </div>
                    <div className="flex items-center gap-4 text-sm font-medium">
                      <span className="text-secondary">{formatBytes(item.file_size)}</span>
                      <span className="w-1 h-1 rounded-full bg-outline-variant/40" />
                      <span className="text-on-surface-variant/70 tabular-nums">7天下载 {item.count_7d} 次</span>
                      <span className="w-1 h-1 rounded-full bg-outline-variant/40" />
                      <span className="text-on-surface-variant/50 tabular-nums">{timeAgo(item.last_download_at)}</span>
                    </div>
                  </div>

                  <div className="flex items-center gap-3">
                    <button
                      className="w-12 h-12 rounded-xl bg-primary/10 text-primary flex items-center justify-center group-hover:bg-primary group-hover:text-on-primary transition-all duration-300 shadow-sm"
                      title="加速下载"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDownloadHistory(item.url);
                      }}
                    >
                      <span className="material-symbols-outlined text-2xl font-bold">download</span>
                    </button>
                    <a
                      href={`/downloads/${item.slug}`}
                      className="px-4 py-2 text-sm font-bold text-secondary hover:bg-secondary/5 rounded-lg transition-colors"
                      onClick={(e) => e.stopPropagation()}
                    >
                      详情
                    </a>
                  </div>
                </div>
                
                <div className="mt-4 pt-4 border-t border-outline-variant/10">
                  <p className="text-xs text-on-surface-variant font-mono truncate" title={item.url}>
                    {item.url}
                  </p>
                </div>
              </div>
            ))
          )}
        </div>
        
        {history.length > 0 && (
          <div className="mt-12 text-center">
            <a 
              href="/downloads" 
              className="inline-flex items-center gap-2 px-8 py-3 rounded-full border border-outline-variant hover:bg-surface-container-high transition-all text-sm font-bold text-on-surface"
            >
              <span className="material-symbols-outlined text-lg">explore</span>
              探索公共资源库
            </a>
          </div>
        )}
      </section>
    </main>
  );
}

