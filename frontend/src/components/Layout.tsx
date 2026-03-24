import { Outlet, Link, useLocation } from 'react-router-dom';

export default function Layout() {
  const location = useLocation();

  return (
    <div className="bg-surface text-on-surface min-h-screen flex flex-col">
      {/* Top Navigation Bar */}
      <nav className="w-full top-0 sticky bg-[#F7F9FF] z-50">
        <div className="flex items-center justify-between px-6 h-16 max-w-7xl mx-auto">
          <div className="flex items-center gap-8">
            <span className="text-lg font-bold text-[#171C22]">云渡</span>
            <div className="hidden md:flex items-center gap-6 font-medium text-sm">
              <Link
                to="/"
                className={`pb-1 transition-colors ${
                  location.pathname === '/'
                    ? 'text-[#171C22] border-b-2 border-[#00631F]'
                    : 'text-[#424753] hover:text-[#0058BB]'
                }`}
              >
                首页
              </Link>
              <Link
                to="/proxydash"
                className={`pb-1 transition-colors ${
                  location.pathname === '/proxydash'
                    ? 'text-[#171C22] border-b-2 border-[#00631F]'
                    : 'text-[#424753] hover:text-[#0058BB]'
                }`}
              >
                历史记录
              </Link>
            </div>
          </div>
        </div>
        {/* Separation Line */}
        <div className="bg-[#F0F4FC] h-[1px] w-full"></div>
      </nav>

      {/* Main Content */}
      <Outlet />

      {/* Footer */}
      <footer className="w-full border-t border-[#C2C6D6]/20 bg-[#F7F9FF] mt-auto">
        <div className="flex justify-between items-center px-8 py-10 max-w-7xl mx-auto">
          <div className="flex items-center gap-4">
            <span className="text-sm font-semibold text-on-surface">云渡</span>
            <span className="text-xs text-[#424753]">© 2024 veegn. All rights reserved.</span>
          </div>
          <div className="flex items-center gap-6">
            <a
              className="flex items-center gap-2 text-xs font-medium text-[#424753] hover:text-[#0058BB] underline-offset-4 hover:underline transition-all"
              href="#"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.43.372.823 1.102.823 2.222 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"></path>
              </svg>
              GitHub
            </a>
          </div>
        </div>
      </footer>
    </div>
  );
}
