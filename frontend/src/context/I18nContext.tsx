import React, { createContext, useContext, useState, useEffect } from 'react';

const translations = {
  zh: {
    // Navigation / Layout
    'nav.home': '首页',
    'nav.history': '历史记录',
    'nav.filebox': '临时文件箱',
    'footer.desc': '高效、稳定的文件代理下载服务，让资源获取更简单。',
    'footer.github': 'GitHub',
    'footer.privacy': '隐私政策',
    'footer.terms': '使用条款',
    'footer.copyright': '© {year} VEEGN • MADE WITH ❤️ FOR RUST',

    // Dashboard Page
    'seo.dashboard.title': '云渡 - HTTP/HTTPS 下载代理与文件加速',
    'seo.dashboard.description': '云渡提供稳定的 HTTP/HTTPS 文件下载代理、断点续传支持和热门下载记录展示，帮助用户更高效地获取公开下载资源。',
    'seo.dashboard.keywords': '下载代理,HTTP下载,HTTPS下载,文件加速,断点续传,下载历史',
    'dashboard.title': '极简代理 极速下载',
    'dashboard.subtitle': '输入任意 HTTP/HTTPS 下载链接，即刻体验无阻断、全速的文件获取。',
    'dashboard.placeholder': 'https://example.com/large-file.zip',
    'dashboard.btn_download': '开始下载',
    'dashboard.hot_downloads': '热门下载',
    'dashboard.view_all_history': '查看全部历史',
    'dashboard.browse_resources': '浏览资源列表',
    'dashboard.loading': '加载中...',
    'dashboard.no_history': '暂无下载记录',
    'dashboard.count_7d': '7 天内下载 {count} 次',
    'dashboard.re_download': '重新下载',
    'dashboard.detail': '详情',

    // ProxyDash (History) Page
    'seo.proxydash.title': '下载历史记录 - 云渡',
    'seo.proxydash.description': '查看云渡最近处理过的热门下载记录，包括文件名、文件大小、最近处理时间和下载热度。',
    'seo.proxydash.keywords': '下载历史,热门下载,文件记录,下载热度',
    'proxydash.title': '下载历史记录',
    'proxydash.subtitle': '管理并重新下载您之前处理过的所有文件记录。列表已按下载热度排序。',
    'proxydash.table.filename': '文件名',
    'proxydash.table.url': '原始链接',
    'proxydash.table.size': '大小',
    'proxydash.table.score': '热度 / 7天下载',
    'proxydash.table.time': '最近处理时间',
    'proxydash.table.action': '操作',
    'proxydash.action.details': '详情',
    'proxydash.action.download': '下载',
    'proxydash.pagination': '显示 1 到 {count}，共 {count} 条记录',

    // FileBox Page
    'seo.filebox.title': '临时文件箱 - 云渡',
    'seo.filebox.description': '云渡临时文件箱提供安全、快捷的文件中转服务。自助上传，保留最长一周，随时随地极速下载。',
    'seo.filebox.keywords': '临时文件箱,文件分享,临时上传,文件有效期,云储存',
    'filebox.title': '临时文件箱',
    'filebox.subtitle': '自主上传临时文件，生成高带宽极速直链。文件最长保存 7 天，到期自动粉碎销毁。',
    'filebox.quota.title': '临时存储空间配额',
    'filebox.quota.used': '已用 {used} / 共 {total} ({percent}%)',
    'filebox.err.fetch': '获取文件列表失败',
    'filebox.err.load': '无法加载文件列表，请稍后重试。',
    'filebox.err.upload_failed': '文件上传失败，空间已满或文件超出限制。',
    'filebox.err.network': '网络错误，上传失败。',
    'filebox.err.delete_failed': '删除文件失败，请重试。',
    'filebox.delete.confirm': '确定要彻底删除该文件吗？',
    'filebox.upload.uploading': '正在上传文件...',
    'filebox.upload.uploading_sub': '请勿关闭当前页面，正在安全流式传输文件',
    'filebox.upload.dragging': '拖拽文件到这里，或点击选择',
    'filebox.upload.dragging_sub': '支持上传任意文件格式，文件最大保存 7 天',
    'filebox.list.title': '当前文件箱',
    'filebox.list.loading': '加载列表中...',
    'filebox.list.empty': '临时文件箱为空',
    'filebox.list.empty_sub': '上传一些文件，生成的极速链接会在此展示',
    'filebox.table.filename': '文件名',
    'filebox.table.size': '大小',
    'filebox.table.expires': '有效期',
    'filebox.table.action': '操作',
    'filebox.action.copy': '复制直链',
    'filebox.action.copied': '已复制',
    'filebox.action.download': '下载',
    'filebox.action.delete': '删除',
    'filebox.expires.expired': '已过期',

    // NotFound (404) Page
    'seo.notfound.title': '404 - 页面不存在 | 云渡',
    'seo.notfound.description': '你访问的页面不存在，将在 5 秒后返回云渡首页。',
    'notfound.subtitle': 'Page Not Found',
    'notfound.title': '404',
    'notfound.text': '你访问的页面不存在',
    'notfound.path': '当前路径：{path}',
    'notfound.countdown': '{secs} 秒后将自动返回首页，你也可以立即手动返回。',
    'notfound.btn_home': '返回首页',
  },
  en: {
    // Navigation / Layout
    'nav.home': 'Home',
    'nav.history': 'History',
    'nav.filebox': 'File Box',
    'footer.desc': 'Efficient and stable file proxy download service, making resource acquisition simpler.',
    'footer.github': 'GitHub',
    'footer.privacy': 'Privacy Policy',
    'footer.terms': 'Terms of Service',
    'footer.copyright': '© {year} VEEGN • MADE WITH ❤️ FOR RUST',

    // Dashboard Page
    'seo.dashboard.title': 'Yundo - HTTP/HTTPS Download Proxy & File Acceleration',
    'seo.dashboard.description': 'Yundo provides stable HTTP/HTTPS file download proxy, breakpoint transmission support, and hot download record displays, helping users acquire public download resources more efficiently.',
    'seo.dashboard.keywords': 'download proxy,HTTP download,HTTPS download,file acceleration,breakpoint transmission,download history',
    'dashboard.title': 'Minimalist Proxy, High-speed Download',
    'dashboard.subtitle': 'Enter any HTTP/HTTPS download link to instantly experience unblocked, full-speed file acquisition.',
    'dashboard.placeholder': 'https://example.com/large-file.zip',
    'dashboard.btn_download': 'Download',
    'dashboard.hot_downloads': 'Trending',
    'dashboard.view_all_history': 'All History',
    'dashboard.browse_resources': 'Browse Resource List',
    'dashboard.loading': 'Loading...',
    'dashboard.no_history': 'No download records yet',
    'dashboard.count_7d': 'Downloaded {count} times in 7 days',
    'dashboard.re_download': 'Re-download',
    'dashboard.detail': 'Details',

    // ProxyDash (History) Page
    'seo.proxydash.title': 'Download History - Yundo',
    'seo.proxydash.description': 'View hot download records recently processed by Yundo, including file name, file size, last processed time, and download popularity.',
    'seo.proxydash.keywords': 'download history,trending downloads,file records,download popularity',
    'proxydash.title': 'Download History',
    'proxydash.subtitle': 'Manage and re-download all file records you have processed before. The list is sorted by download popularity.',
    'proxydash.table.filename': 'File Name',
    'proxydash.table.url': 'Original URL',
    'proxydash.table.size': 'Size',
    'proxydash.table.score': 'Score / 7D Downloads',
    'proxydash.table.time': 'Last Processed',
    'proxydash.table.action': 'Action',
    'proxydash.action.details': 'Details',
    'proxydash.action.download': 'Download',
    'proxydash.pagination': 'Showing 1 to {count} of {count} records',

    // FileBox Page
    'seo.filebox.title': 'Temporary File Bin - Yundo',
    'seo.filebox.description': 'Yundo temporary file bin provides secure and quick file transfer services. Self-upload, retained for up to a week, download at full speed anytime, anywhere.',
    'seo.filebox.keywords': 'temporary file bin,file sharing,temporary upload,file expiration,cloud storage',
    'filebox.title': 'Temporary File Bin',
    'filebox.subtitle': 'Upload temporary files, generate high-bandwidth speed links. Files are kept for at most 7 days and automatically shredded upon expiration.',
    'filebox.quota.title': 'Temporary Storage Quota',
    'filebox.quota.used': '{used} of {total} used ({percent}%)',
    'filebox.err.fetch': 'Failed to fetch file list',
    'filebox.err.load': 'Unable to load file list. Please try again later.',
    'filebox.err.upload_failed': 'File upload failed. Space quota exceeded or file limit reached.',
    'filebox.err.network': 'Network error. Upload failed.',
    'filebox.err.delete_failed': 'Failed to delete file. Please try again.',
    'filebox.delete.confirm': 'Are you sure you want to completely delete this file?',
    'filebox.upload.uploading': 'Uploading file...',
    'filebox.upload.uploading_sub': 'Do not close this page. Files are being securely streamed',
    'filebox.upload.dragging': 'Drag & drop files here, or click to browse',
    'filebox.upload.dragging_sub': 'Supports uploading any file format, files are kept for at most 7 days',
    'filebox.list.title': 'Current Files',
    'filebox.list.loading': 'Loading list...',
    'filebox.list.empty': 'Temporary file bin is empty',
    'filebox.list.empty_sub': 'Upload some files, and the generated speed links will be displayed here',
    'filebox.table.filename': 'File Name',
    'filebox.table.size': 'Size',
    'filebox.table.expires': 'Expires',
    'filebox.table.action': 'Action',
    'filebox.action.copy': 'Copy Link',
    'filebox.action.copied': 'Copied',
    'filebox.action.download': 'Download',
    'filebox.action.delete': 'Delete',
    'filebox.expires.expired': 'Expired',

    // NotFound (404) Page
    'seo.notfound.title': '404 - Page Not Found | Yundo',
    'seo.notfound.description': 'The page you visited does not exist. You will be redirected to the home page in 5 seconds.',
    'notfound.subtitle': 'Page Not Found',
    'notfound.title': '404',
    'notfound.text': 'The page you are looking for does not exist.',
    'notfound.path': 'Current path: {path}',
    'notfound.countdown': 'Will automatically return to home page in {secs} seconds, or you can manually return now.',
    'notfound.btn_home': 'Go Home',
  }
};

export type Locale = 'zh' | 'en';
export type TranslationKey = keyof typeof translations.zh;

interface I18nContextType {
  locale: Locale;
  changeLanguage: (lang: Locale) => void;
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
}

const I18nContext = createContext<I18nContextType | null>(null);

const STORAGE_KEY = 'yundo-locale';

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocale] = useState<Locale>(() => {
    // 1. Check local storage
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === 'zh' || saved === 'en') {
      return saved as Locale;
    }
    // 2. Auto-detect browser language
    const lang = navigator.language || '';
    if (lang.toLowerCase().startsWith('zh')) {
      return 'zh';
    }
    return 'en';
  });

  const changeLanguage = (lang: Locale) => {
    setLocale(lang);
    localStorage.setItem(STORAGE_KEY, lang);
    // Update HTML lang attribute for accessibility/SEO
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
  };

  useEffect(() => {
    document.documentElement.lang = locale === 'zh' ? 'zh-CN' : 'en';
  }, [locale]);

  const t = (key: TranslationKey, params?: Record<string, string | number>): string => {
    let text = translations[locale][key] || translations['en'][key] || key;
    if (params) {
      Object.entries(params).forEach(([k, v]) => {
        text = text.replace(new RegExp(`{${k}}`, 'g'), String(v));
      });
    }
    return text;
  };

  return (
    <I18nContext.Provider value={{ locale, changeLanguage, t }}>
      {children}
    </I18nContext.Provider>
  );
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) {
    throw new Error('useI18n must be used within an I18nProvider');
  }
  return context;
}
