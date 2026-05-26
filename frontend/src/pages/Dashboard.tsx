import { useState } from 'react';
import { withBasePath } from '../utils/basePath';
import { useI18n } from '../context/I18nContext';

export default function Dashboard() {
  const [url, setUrl] = useState('');
  const { locale, t } = useI18n();





  const handleDownload = () => {
    if (!url) return;
    window.open(`${withBasePath('/api/proxy')}?url=${encodeURIComponent(url)}`, '_blank');
  };



  return (
    <main className="flex-grow flex flex-col items-center px-6 pt-24 pb-16 max-w-7xl mx-auto w-full">
      <section className="w-full max-w-2xl text-center mb-24">
        <h1 className="text-4xl font-extrabold tracking-tight text-on-surface mb-4">
          {t('dashboard.title')}
        </h1>
        <p className="text-on-surface-variant mb-12">
          {t('dashboard.subtitle')}
        </p>
        <div className="flex flex-col md:flex-row gap-3 p-2 bg-surface-container-lowest rounded-xl ghost-border ghost-shadow">
          <div className="flex-grow flex items-center px-4 bg-surface-container-lowest">
            <span className="material-symbols-outlined text-outline mr-3">link</span>
            <input
              className="w-full bg-transparent border-none focus:ring-0 text-on-surface placeholder:text-outline/60 py-3 font-medium outline-none"
              placeholder={t('dashboard.placeholder')}
              type="text"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && handleDownload()}
            />
          </div>
          <button
            onClick={handleDownload}
            className="bg-primary-gradient text-on-primary px-8 py-3 rounded-lg font-semibold flex items-center justify-center gap-2 hover:opacity-90 active:scale-[0.98] transition-all cursor-pointer"
          >
            <span>{t('dashboard.btn_download')}</span>
            <span className="material-symbols-outlined text-sm">rocket_launch</span>
          </button>
        </div>
      </section>


    </main>
  );
}

