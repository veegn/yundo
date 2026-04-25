import { BrowserRouter, Routes, Route } from 'react-router-dom';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import ProxyDash from './pages/ProxyDash';
import NotFound from './pages/NotFound';
import { getBasePath } from './utils/basePath';

export default function App() {
  const basePath = getBasePath();

  return (
    <BrowserRouter basename={basePath === '/' ? undefined : basePath}>
      <Routes>
        <Route path="/" element={<Layout />}>
          <Route index element={<Dashboard />} />
          <Route path="proxydash" element={<ProxyDash />} />
          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
