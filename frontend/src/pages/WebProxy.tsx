import { useState } from 'react';
import { withBasePath } from '../utils/basePath';
import { useI18n } from '../context/I18nContext';

export default function WebProxy() {
  const [url, setUrl] = useState('');
  const [error, setError] = useState('');
  const { t } = useI18n();

  const openProxy = () => {
    const trimmed = url.trim();
    if (!trimmed) return;
    if (!trimmed.startsWith('http://') && !trimmed.startsWith('https://')) {
      setError(t('webproxy.err_invalid'));
      return;
    }

    setError('');
    window.open(`${withBasePath('/browse')}?url=${encodeURIComponent(trimmed)}`, '_blank');
  };

  return (
    <main className="flex-grow flex flex-col items-center px-6 pt-24 pb-16 max-w-7xl mx-auto w-full">
      <section className="w-full max-w-2xl text-center mb-14">
        <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-4">
          {t('webproxy.title')}
        </h1>
        <p className="text-on-surface-variant mb-10">
          {t('webproxy.subtitle')}
        </p>
        <div className="flex flex-col md:flex-row gap-3 p-2 bg-surface-container-lowest rounded-xl ghost-border ghost-shadow">
          <div className="flex-grow flex items-center px-4 bg-surface-container-lowest">
            <span className="material-symbols-outlined text-outline mr-3">travel_explore</span>
            <input
              className="w-full bg-transparent border-none focus:ring-0 text-on-surface placeholder:text-outline/60 py-3 font-medium outline-none"
              placeholder="https://example.com"
              type="text"
              value={url}
              onChange={(event) => {
                setUrl(event.target.value);
                setError('');
              }}
              onKeyDown={(event) => event.key === 'Enter' && openProxy()}
            />
          </div>
          <button
            onClick={openProxy}
            className="bg-primary-gradient text-on-primary px-8 py-3 rounded-lg font-semibold flex items-center justify-center gap-2 hover:opacity-90 active:scale-[0.98] transition-all cursor-pointer"
          >
            <span>{t('webproxy.btn_open')}</span>
            <span className="material-symbols-outlined text-sm">open_in_new</span>
          </button>
        </div>
        {error && <p className="mt-4 text-sm font-medium text-error">{error}</p>}
      </section>

      <section className="w-full max-w-3xl bg-surface-container-low rounded-xl p-6 text-left">
        <h2 className="text-lg font-bold text-on-surface mb-3">{t('webproxy.notice_title')}</h2>
        <div className="space-y-2 text-sm leading-6 text-on-surface-variant">
          <p>{t('webproxy.notice_cookie')}</p>
          <p>{t('webproxy.notice_limit')}</p>
          <p>{t('webproxy.notice_login')}</p>
        </div>
      </section>
    </main>
  );
}
