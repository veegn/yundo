import { BrowserRouter, Routes, Route } from 'react-router-dom';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import ProxyDash from './pages/ProxyDash';
import FileBox from './pages/FileBox';
import WebProxy from './pages/WebProxy';
import Privacy from './pages/Privacy';
import Terms from './pages/Terms';
import NotFound from './pages/NotFound';
import { getBasePath } from './utils/basePath';
import { I18nProvider } from './context/I18nContext';

export default function App() {
  const basePath = getBasePath();

  return (
    <I18nProvider>
      <BrowserRouter basename={basePath === '/' ? undefined : basePath}>
        <Routes>
          <Route path="/" element={<Layout />}>
            <Route index element={<Dashboard />} />
            <Route path="proxydash" element={<ProxyDash />} />
            <Route path="filebox" element={<FileBox />} />
            <Route path="webproxy" element={<WebProxy />} />
            <Route path="privacy" element={<Privacy />} />
            <Route path="terms" element={<Terms />} />
            <Route path="*" element={<NotFound />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </I18nProvider>
  );
}
