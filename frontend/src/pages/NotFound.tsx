import { useEffect, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';

export default function NotFound() {
  const navigate = useNavigate();
  const location = useLocation();
  const [countdown, setCountdown] = useState(5);

  useEffect(() => {
    const interval = window.setInterval(() => {
      setCountdown((current) => {
        if (current <= 1) {
          window.clearInterval(interval);
          navigate('/', { replace: true });
          return 0;
        }
        return current - 1;
      });
    }, 1000);

    return () => window.clearInterval(interval);
  }, [navigate]);

  return (
    <main className="flex-grow flex items-center justify-center px-6 py-16">
      <section className="w-full max-w-xl rounded-[28px] border border-outline-variant/20 bg-surface-container-lowest px-8 py-12 text-center shadow-[0_24px_80px_rgba(23,28,34,0.08)]">
        <p className="mb-3 text-sm font-semibold uppercase tracking-[0.24em] text-secondary">Page Not Found</p>
        <h1 className="mb-4 text-6xl font-black tracking-tight text-primary">404</h1>
        <p className="mb-3 text-lg font-semibold text-on-surface">你访问的页面不存在</p>
        <p className="mb-3 break-all rounded-full bg-secondary-fixed px-4 py-2 text-xs font-medium text-secondary">
          当前路径：{location.pathname}
          {location.search}
        </p>
        <p className="mb-8 text-sm leading-7 text-on-surface-variant">
          {countdown} 秒后将自动返回首页，你也可以立即手动返回。
        </p>
        <Link
          to="/"
          className="inline-flex items-center justify-center rounded-xl bg-primary-gradient px-6 py-3 text-sm font-semibold text-on-primary transition-opacity hover:opacity-90"
        >
          返回首页
        </Link>
      </section>
    </main>
  );
}
