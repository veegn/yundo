import { Outlet, Link, useLocation } from 'react-router-dom';

export default function Layout() {
  const location = useLocation();
  const currentYear = new Date().getFullYear();

  return (
    <div className="bg-surface text-on-surface min-h-screen flex flex-col font-sans selection:bg-primary-container selection:text-on-primary-container">
      <nav className="w-full top-0 sticky z-50 glass">
        <div className="flex items-center justify-between px-6 h-16 max-w-7xl mx-auto">
          <div className="flex items-center gap-10">
            <Link to="/" className="flex items-center gap-2 group">
              <div className="w-8 h-8 bg-primary-gradient rounded-lg flex items-center justify-center text-on-primary shadow-sm group-hover:rotate-12 transition-transform">
                <span className="material-symbols-outlined text-lg">sailing</span>
              </div>
              <span className="text-xl font-bold tracking-tight text-on-surface">云渡</span>
            </Link>
            <div className="hidden md:flex items-center gap-8 font-medium text-sm">
              <Link
                to="/"
                className={`transition-all relative py-1 ${
                  location.pathname === '/'
                    ? 'text-on-surface'
                    : 'text-on-surface-variant hover:text-primary'
                }`}
              >
                首页
                {location.pathname === '/' && (
                  <span className="absolute bottom-0 left-0 w-full h-0.5 bg-primary rounded-full" />
                )}
              </Link>
              <Link
                to="/proxydash"
                className={`transition-all relative py-1 ${
                  location.pathname === '/proxydash'
                    ? 'text-on-surface'
                    : 'text-on-surface-variant hover:text-primary'
                }`}
              >
                历史记录
                {location.pathname === '/proxydash' && (
                  <span className="absolute bottom-0 left-0 w-full h-0.5 bg-primary rounded-full" />
                )}
              </Link>
            </div>
          </div>
          <div className="flex items-center gap-4">
            <a
              href="https://github.com/veegn/yundo"
              target="_blank"
              rel="noreferrer"
              className="p-2 rounded-full hover:bg-surface-container transition-colors text-on-surface-variant"
              title="GitHub Repository"
            >
              <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.43.372.823 1.102.823 2.222 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"></path>
              </svg>
            </a>
          </div>
        </div>
      </nav>

      <Outlet />

      <footer className="w-full border-t border-outline-variant/10 bg-surface-container-low mt-auto backdrop-blur-md">
        <div className="flex flex-col md:flex-row justify-between items-center px-8 py-12 max-w-7xl mx-auto gap-8">
          <div className="flex flex-col items-center md:items-start gap-3">
            <div className="flex items-center gap-2">
              <div className="w-6 h-6 bg-primary-gradient rounded flex items-center justify-center text-on-primary transform rotate-6">
                <span className="material-symbols-outlined text-[14px]">sailing</span>
              </div>
              <span className="text-lg font-bold text-on-surface">云渡</span>
            </div>
            <p className="text-sm text-on-surface-variant max-w-xs text-center md:text-left">
              高效、稳定的文件代理下载服务，让资源获取更简单。
            </p>
          </div>
          <div className="flex flex-col items-center md:items-end gap-4">
            <div className="flex items-center gap-6">
              <a href="https://github.com/veegn/yundo" className="text-sm font-medium text-on-surface-variant hover:text-primary transition-colors">GitHub</a>
              <a href="#" className="text-sm font-medium text-on-surface-variant hover:text-primary transition-colors">隐私政策</a>
              <a href="#" className="text-sm font-medium text-on-surface-variant hover:text-primary transition-colors">使用条款</a>
            </div>
            <span className="text-xs text-on-surface-variant/60 tracking-wider font-mono">
              © {currentYear} VEEGN • MADE WITH ❤️ FOR RUST
            </span>
          </div>
        </div>
      </footer>
    </div>
  );
}

