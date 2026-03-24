import { useState, useEffect } from 'react';
import { formatBytes, formatDate, getIconForFileName } from '../utils/formatters';

interface HistoryItem {
  url: string;
  file_name: string;
  file_size: number;
  last_download_at: string;
  count_7d: number;
  score: number;
}

export default function ProxyDash() {
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    fetch('/api/recent')
      .then(res => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then(data => {
        setHistory(data);
        setLoading(false);
      })
      .catch(err => {
        console.error("Failed to fetch history:", err);
        setLoading(false);
      });
  }, []);

  const handleDownload = (url: string) => {
    const proxyUrl = `/api/proxy?url=${encodeURIComponent(url)}`;
    window.open(proxyUrl, '_blank');
  };

  return (
    <main className="flex-grow max-w-7xl w-full mx-auto px-6 py-12">
      {/* Page Header */}
      <div className="mb-10">
        <h1 className="text-3xl font-bold tracking-tight text-on-surface mb-2">转换历史记录</h1>
        <p className="text-on-surface-variant text-sm">
          管理并重新下载您之前处理过的所有文件记录。列表已按下载热度排序。
        </p>
      </div>

      {/* Table Section */}
      <div className="bg-surface-container-lowest rounded-xl overflow-hidden border border-outline-variant/20 shadow-sm">
        <div className="overflow-x-auto">
          <table className="w-full text-left border-collapse">
            <thead>
              <tr className="bg-surface-container-low border-b border-outline-variant/10">
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider">
                  文件名
                </th>
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider">
                  原始链接
                </th>
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider">
                  大小
                </th>
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider">
                  热度 / 7天下载
                </th>
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider">
                  最近处理时间
                </th>
                <th className="px-6 py-4 text-xs font-semibold text-on-surface-variant uppercase tracking-wider text-right">
                  操作
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-outline-variant/10">
              {loading ? (
                <tr>
                  <td colSpan={6} className="px-6 py-8 text-center text-on-surface-variant">
                    加载中...
                  </td>
                </tr>
              ) : history.length === 0 ? (
                <tr>
                  <td colSpan={6} className="px-6 py-8 text-center text-on-surface-variant">
                    暂无下载记录
                  </td>
                </tr>
              ) : (
                history.map((item, index) => (
                  <tr key={index} className="hover:bg-surface-container transition-colors group">
                    <td className="px-6 py-5">
                      <div className="flex items-center gap-3">
                        <span className="material-symbols-outlined text-secondary">
                          {getIconForFileName(item.file_name)}
                        </span>
                        <span className="font-medium text-sm text-on-surface truncate max-w-[200px]" title={item.file_name}>
                          {item.file_name}
                        </span>
                        {index === 0 && <span title="最热下载">🔥</span>}
                      </div>
                    </td>
                    <td className="px-6 py-5">
                      <span className="text-sm text-on-surface-variant font-mono truncate max-w-[200px] lg:max-w-xs block" title={item.url}>
                        {item.url}
                      </span>
                    </td>
                    <td className="px-6 py-5">
                      <span className="text-sm tabular-nums text-on-surface-variant">
                        {formatBytes(item.file_size)}
                      </span>
                    </td>
                    <td className="px-6 py-5">
                      <div className="flex flex-col">
                        <span className="text-sm text-on-surface-variant">
                          {item.score.toFixed(2)}
                        </span>
                        <span className="text-[10px] text-on-surface-variant/70">
                          {item.count_7d} 次
                        </span>
                      </div>
                    </td>
                    <td className="px-6 py-5">
                      <span className="text-sm text-on-surface-variant">
                        {formatDate(item.last_download_at)}
                      </span>
                    </td>
                    <td className="px-6 py-5 text-right">
                      <button 
                        onClick={() => handleDownload(item.url)}
                        className="inline-flex items-center gap-2 px-4 py-2 bg-gradient-to-b from-primary-container to-primary text-on-primary text-xs font-semibold rounded-lg hover:opacity-90 transition-all shadow-sm"
                      >
                        <span
                          className="material-symbols-outlined text-sm"
                          style={{ fontVariationSettings: "'FILL' 1" }}
                        >
                          download
                        </span>
                        下载
                      </button>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>

        {/* Pagination (Minimalist) */}
        {!loading && history.length > 0 && (
          <div className="px-6 py-4 bg-surface-container-low flex items-center justify-between border-t border-outline-variant/10">
            <p className="text-xs text-on-surface-variant">显示 1 到 {history.length} 共 {history.length} 条记录</p>
          </div>
        )}
      </div>
    </main>
  );
}
