import React, { useState, useEffect, useRef } from 'react';
import { formatBytes, getIconForFileName } from '../utils/formatters';
import { useSeo } from '../utils/seo';
import { withBasePath } from '../utils/basePath';

interface FileBoxItem {
  id: string;
  file_name: string;
  file_size: number;
  uploaded_at: string;
  expires_at: string;
}

interface Stats {
  total_space: number;
  used_space: number;
  files: FileBoxItem[];
}

export default function FileBox() {
  const [stats, setStats] = useState<Stats>({
    total_space: 5 * 1024 * 1024 * 1024,
    used_space: 0,
    files: [],
  });
  const [loading, setLoading] = useState(true);
  const [uploading, setUploading] = useState(false);
  const [uploadProgress, setUploadProgress] = useState(0);
  const [dragActive, setDragActive] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  useSeo({
    title: '临时文件箱 - 云渡',
    description:
      '云渡临时文件箱提供安全、快捷的文件中转服务。自助上传，保留最长一周，随时随地极速下载。',
    canonicalPath: '/filebox',
    keywords: '临时文件箱,文件分享,临时上传,文件有效期,云储存',
  });

  const fetchFiles = () => {
    setLoading(true);
    fetch(withBasePath('/api/filebox/files'))
      .then((res) => {
        if (!res.ok) throw new Error(`获取文件列表失败: ${res.status}`);
        return res.json();
      })
      .then((data) => {
        setStats(data);
        setLoading(false);
      })
      .catch((err) => {
        console.error(err);
        setErrorMessage('无法加载文件列表，请稍后重试。');
        setLoading(false);
      });
  };

  useEffect(() => {
    fetchFiles();
  }, []);

  const handleDrag = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.type === 'dragenter' || e.type === 'dragover') {
      setDragActive(true);
    } else if (e.type === 'dragleave') {
      setDragActive(false);
    }
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragActive(false);
    if (e.dataTransfer.files && e.dataTransfer.files[0]) {
      uploadFiles(e.dataTransfer.files);
    }
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files[0]) {
      uploadFiles(e.target.files);
    }
  };

  const uploadFiles = (files: FileList) => {
    setUploading(true);
    setErrorMessage(null);
    setUploadProgress(0);

    const formData = new FormData();
    for (let i = 0; i < files.length; i++) {
      formData.append('files', files[i]);
    }

    const xhr = new XMLHttpRequest();
    xhr.open('POST', withBasePath('/api/filebox/upload'), true);

    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) {
        const percent = Math.round((e.loaded / e.total) * 100);
        setUploadProgress(percent);
      }
    };

    xhr.onload = () => {
      setUploading(false);
      if (xhr.status === 200) {
        fetchFiles();
      } else {
        try {
          const res = JSON.parse(xhr.responseText);
          setErrorMessage(res.message || '文件上传失败，空间已满或文件超出限制。');
        } catch {
          setErrorMessage('文件上传失败，空间已满或文件超出限制。');
        }
      }
    };

    xhr.onerror = () => {
      setUploading(false);
      setErrorMessage('网络错误，上传失败。');
    };

    xhr.send(formData);
  };

  const handleDelete = (id: string) => {
    if (!confirm('确定要彻底删除该文件吗？')) return;

    fetch(withBasePath(`/api/filebox/delete/${id}`), { method: 'DELETE' })
      .then((res) => {
        if (!res.ok) throw new Error('删除失败');
        fetchFiles();
      })
      .catch((err) => {
        console.error(err);
        setErrorMessage('删除文件失败，请重试。');
      });
  };

  const copyToClipboard = (id: string) => {
    const downloadUrl = `${window.location.origin}${withBasePath(`/api/filebox/download/${id}`)}`;
    navigator.clipboard
      .writeText(downloadUrl)
      .then(() => {
        setCopiedId(id);
        setTimeout(() => setCopiedId(null), 2000);
      })
      .catch((err) => console.error('复制链接失败: ', err));
  };

  const formatExpiration = (expiresAtString: string) => {
    const expires = new Date(expiresAtString.replace(' ', 'T') + 'Z');
    const now = new Date();
    const diffMs = expires.getTime() - now.getTime();
    if (diffMs <= 0) return '已过期';

    const diffSecs = Math.floor(diffMs / 1000);
    const days = Math.floor(diffSecs / 86400);
    const hours = Math.floor((diffSecs % 86400) / 3600);
    const mins = Math.floor((diffSecs % 3600) / 60);

    if (days > 0) {
      return `剩 ${days} 天 ${hours} 小时`;
    }
    if (hours > 0) {
      return `剩 ${hours} 小时 ${mins} 分钟`;
    }
    return `剩 ${mins} 分钟`;
  };

  const getExpirationBadgeClass = (expiresAtString: string) => {
    const expires = new Date(expiresAtString.replace(' ', 'T') + 'Z');
    const now = new Date();
    const diffMs = expires.getTime() - now.getTime();
    const diffDays = diffMs / (1000 * 60 * 60 * 24);

    if (diffDays < 1) {
      return 'bg-red-100 text-red-700 border border-red-200';
    } else if (diffDays < 3) {
      return 'bg-yellow-100 text-yellow-800 border border-yellow-200';
    }
    return 'bg-green-100 text-green-700 border border-green-200';
  };

  const usedPercentage = Math.min((stats.used_space / stats.total_space) * 100, 100);

  return (
    <main className="flex-grow flex flex-col items-center px-6 pt-24 pb-16 max-w-7xl mx-auto w-full">
      <section className="w-full max-w-4xl text-center mb-12">
        <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-4">
          临时文件箱
        </h1>
        <p className="text-on-surface-variant max-w-xl mx-auto mb-8">
          自主上传临时文件，生成高带宽极速直链。文件最长保存 <span className="font-bold text-secondary">7 天</span>，到期自动粉碎销毁。
        </p>

        {/* Storage usage bar */}
        <div className="bg-surface-container-low p-6 rounded-2xl ghost-border ghost-shadow text-left mb-8">
          <div className="flex justify-between items-center mb-3">
            <span className="text-sm font-semibold text-on-surface flex items-center gap-1.5">
              <span className="material-symbols-outlined text-secondary text-lg">cloud_queue</span>
              临时存储空间配额
            </span>
            <span className="text-sm font-mono text-on-surface-variant">
              {formatBytes(stats.used_space)} / {formatBytes(stats.total_space)} ({usedPercentage.toFixed(1)}%)
            </span>
          </div>
          <div className="w-full bg-surface-container-high h-3.5 rounded-full overflow-hidden">
            <div
              className={`h-full transition-all duration-500 ease-out rounded-full ${
                usedPercentage > 90
                  ? 'bg-red-500'
                  : usedPercentage > 70
                  ? 'bg-yellow-500'
                  : 'bg-primary-gradient'
              }`}
              style={{ width: `${usedPercentage}%` }}
            />
          </div>
        </div>

        {errorMessage && (
          <div className="bg-red-50 text-red-800 p-4 rounded-xl border border-red-200 text-left mb-6 flex items-start gap-2">
            <span className="material-symbols-outlined shrink-0">error</span>
            <span className="text-sm font-medium">{errorMessage}</span>
          </div>
        )}

        {/* Drag and Drop Zone */}
        <div
          onDragEnter={handleDrag}
          onDragOver={handleDrag}
          onDragLeave={handleDrag}
          onDrop={handleDrop}
          onClick={() => fileInputRef.current?.click()}
          className={`relative border-2 border-dashed rounded-2xl p-12 text-center cursor-pointer transition-all duration-300 ${
            dragActive
              ? 'border-secondary bg-secondary/5 scale-[1.01]'
              : 'border-outline-variant/60 hover:border-secondary hover:bg-surface-container-low'
          } ${uploading ? 'pointer-events-none' : ''}`}
        >
          <input
            ref={fileInputRef}
            type="file"
            multiple
            className="hidden"
            onChange={handleFileChange}
          />
          {uploading ? (
            <div className="flex flex-col items-center">
              <div className="relative w-20 h-20 mb-4 flex items-center justify-center">
                {/* Circular spinner */}
                <div className="absolute inset-0 border-4 border-secondary-fixed rounded-full"></div>
                <div
                  className="absolute inset-0 border-4 border-secondary rounded-full border-t-transparent animate-spin"
                  style={{ animationDuration: '0.8s' }}
                ></div>
                <span className="text-sm font-bold text-secondary font-mono">{uploadProgress}%</span>
              </div>
              <p className="text-on-surface font-semibold mb-1">正在上传文件...</p>
              <p className="text-xs text-on-surface-variant">请勿关闭当前页面，正在安全流式传输文件</p>
            </div>
          ) : (
            <div className="flex flex-col items-center">
              <div className={`w-16 h-16 rounded-2xl bg-secondary-fixed flex items-center justify-center text-on-secondary-fixed mb-4 transition-transform duration-300 ${dragActive ? 'scale-110 rotate-3' : ''}`}>
                <span className="material-symbols-outlined text-3xl">upload_file</span>
              </div>
              <h3 className="text-lg font-bold text-on-surface mb-1">拖拽文件到这里，或点击选择</h3>
              <p className="text-sm text-on-surface-variant">支持上传任意文件格式，文件最大保存 7 天</p>
            </div>
          )}
        </div>
      </section>

      {/* Files List */}
      <section className="w-full max-w-4xl">
        <h2 className="text-xl font-bold text-on-surface mb-6 flex items-center gap-2">
          <span>当前文件箱</span>
          <span className="bg-secondary-fixed text-on-secondary-fixed text-xs px-2.5 py-1 rounded-full font-mono font-bold">
            {stats.files.length}
          </span>
        </h2>

        <div className="bg-surface-container-low rounded-xl overflow-hidden shadow-sm ghost-border">
          {loading ? (
            <div className="p-12 text-center text-on-surface-variant flex flex-col items-center gap-2">
              <div className="w-8 h-8 border-4 border-outline-variant border-t-secondary rounded-full animate-spin"></div>
              <span className="text-sm mt-2">加载列表中...</span>
            </div>
          ) : stats.files.length === 0 ? (
            <div className="p-16 text-center text-on-surface-variant">
              <span className="material-symbols-outlined text-5xl opacity-40 mb-3 block">folder_open</span>
              <p className="text-base font-medium">临时文件箱为空</p>
              <p className="text-xs mt-1 text-on-surface-variant/70">上传一些文件，生成的极速链接会在此展示</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-left border-collapse">
                <thead>
                  <tr className="bg-surface-container border-b border-outline-variant/10 text-on-surface-variant/80 text-xs font-semibold uppercase tracking-wider">
                    <th className="px-6 py-4">文件名</th>
                    <th className="px-6 py-4">大小</th>
                    <th className="px-6 py-4">有效期</th>
                    <th className="px-6 py-4 text-right">操作</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-outline-variant/10">
                  {stats.files.map((file) => (
                    <tr
                      key={file.id}
                      className="hover:bg-surface-container-high transition-colors bg-surface-container-lowest"
                    >
                      <td className="px-6 py-4 max-w-xs sm:max-w-md">
                        <div className="flex items-center gap-3">
                          <div className="w-9 h-9 rounded bg-secondary-fixed flex items-center justify-center shrink-0">
                            <span className="material-symbols-outlined text-on-secondary-fixed text-lg">
                              {getIconForFileName(file.file_name)}
                            </span>
                          </div>
                          <div className="truncate">
                            <span
                              className="font-bold text-sm text-on-surface hover:text-[#0058BB] cursor-pointer"
                              onClick={() => window.open(withBasePath(`/api/filebox/download/${file.id}`), '_blank')}
                            >
                              {file.file_name}
                            </span>
                          </div>
                        </div>
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-on-surface-variant font-mono">
                        {formatBytes(file.file_size)}
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap">
                        <span className={`text-[11px] font-bold px-2 py-1 rounded-md ${getExpirationBadgeClass(file.expires_at)}`}>
                          {formatExpiration(file.expires_at)}
                        </span>
                      </td>
                      <td className="px-6 py-4 whitespace-nowrap text-right text-sm font-medium">
                        <div className="flex justify-end gap-1.5">
                          {/* Copy link button */}
                          <div className="relative">
                            <button
                              onClick={() => copyToClipboard(file.id)}
                              className="p-1.5 hover:bg-surface-container-high text-on-surface-variant hover:text-secondary rounded-lg transition-colors flex items-center gap-1"
                              title="复制下载直链"
                            >
                              <span className="material-symbols-outlined text-[19px]">
                                {copiedId === file.id ? 'check_circle' : 'link'}
                              </span>
                              <span className="text-xs">{copiedId === file.id ? '已复制' : '复制直链'}</span>
                            </button>
                          </div>
                          
                          {/* Download button */}
                          <a
                            href={withBasePath(`/api/filebox/download/${file.id}`)}
                            target="_blank"
                            rel="noreferrer"
                            className="p-1.5 hover:bg-surface-container-high text-on-surface-variant hover:text-primary rounded-lg transition-colors flex items-center gap-1"
                            title="下载"
                          >
                            <span className="material-symbols-outlined text-[19px]">download</span>
                            <span className="text-xs">下载</span>
                          </a>

                          {/* Delete button */}
                          <button
                            onClick={() => handleDelete(file.id)}
                            className="p-1.5 hover:bg-surface-container-high text-on-surface-variant hover:text-error rounded-lg transition-colors flex items-center gap-1"
                            title="删除"
                          >
                            <span className="material-symbols-outlined text-[19px]">delete</span>
                            <span className="text-xs">删除</span>
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </section>
    </main>
  );
}
