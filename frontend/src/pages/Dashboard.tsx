import { Link } from 'react-router-dom';
import { useState, useEffect } from 'react';
import { formatBytes, timeAgo, getIconForFileName } from '../utils/formatters';

interface HistoryItem {
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

  useEffect(() => {
    fetch('/api/recent')
      .then(res => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then(data => {
        setHistory(data.slice(0, 3)); // Only show top 3 on dashboard
        setLoading(false);
      })
      .catch(err => {
        console.error("Failed to fetch history:", err);
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
    <main className="flex-grow flex flex-col items-center px-6 pt-24 pb-16 max-w-7xl mx-auto w-full">
      {/* Hero Input Section */}
      <section className="w-full max-w-2xl text-center mb-24">
        <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-4">
          极简代理 极速下载
        </h1>
        <p className="text-on-surface-variant mb-12">
          输入任意 HTTP/HTTPS 下载链接，即刻体验无阻断、全速的文件获取
        </p>
        <div className="flex flex-col md:flex-row gap-3 p-2 bg-surface-container-lowest rounded-xl ghost-border ghost-shadow">
          <div className="flex-grow flex items-center px-4 bg-surface-container-lowest">
            <span className="material-symbols-outlined text-outline mr-3">link</span>
            <input
              className="w-full bg-transparent border-none focus:ring-0 text-on-surface placeholder:text-outline/60 py-3 font-medium outline-none"
              placeholder="https://example.com/large-file.zip"
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleDownload()}
            />
          </div>
          <button 
            onClick={handleDownload}
            className="bg-primary-gradient text-on-primary px-8 py-3 rounded-lg font-semibold flex items-center justify-center gap-2 hover:opacity-90 active:scale-[0.98] transition-all"
          >
            <span>开始下载</span>
            <span className="material-symbols-outlined text-sm">rocket_launch</span>
          </button>
        </div>
      </section>

      {/* Activity Feed Section */}
      <section className="w-full max-w-4xl">
        <div className="flex justify-between items-end mb-6">
          <h2 className="text-xl font-bold text-on-surface">热门下载</h2>
          <Link
            to="/proxydash"
            className="text-sm font-medium text-secondary flex items-center gap-1 hover:underline underline-offset-4 transition-all"
          >
            查看全部历史
            <span className="material-symbols-outlined text-xs">arrow_forward</span>
          </Link>
        </div>

        {/* List Container */}
        <div className="bg-surface-container-low rounded-xl overflow-hidden">
          <div className="divide-y divide-outline-variant/10">
            {loading ? (
              <div className="p-8 text-center text-on-surface-variant">加载中...</div>
            ) : history.length === 0 ? (
              <div className="p-8 text-center text-on-surface-variant">暂无下载记录</div>
            ) : (
              history.map((item, index) => (
                <div key={index} className="flex items-center justify-between p-5 hover:bg-surface-container-high transition-colors bg-surface-container-lowest group cursor-pointer" onClick={() => handleDownloadHistory(item.url)}>
                  <div className="flex items-center gap-4 overflow-hidden">
                    <div className="w-10 h-10 rounded bg-secondary-fixed flex items-center justify-center shrink-0 relative">
                      <span className="material-symbols-outlined text-on-secondary-fixed">
                        {getIconForFileName(item.file_name)}
                      </span>
                      {index === 0 && (
                        <span className="absolute -top-2 -right-2 text-lg" title="最热下载">🔥</span>
                      )}
                    </div>
                    <div className="overflow-hidden">
                      <h3 className="font-bold text-sm text-on-surface truncate" title={item.file_name}>{item.file_name}</h3>
                      <p className="text-xs text-on-surface-variant font-mono truncate max-w-[200px] sm:max-w-xs md:max-w-md" title={item.url}>
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
                        7天内下载 {item.count_7d} 次
                      </span>
                    </div>
                    <span className="text-xs text-on-surface-variant font-mono tabular-nums w-20 text-right">
                      {timeAgo(item.last_download_at)}
                    </span>
                    <button 
                      className="w-8 h-8 rounded-full bg-primary-container text-on-primary-container flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                      title="重新下载"
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
