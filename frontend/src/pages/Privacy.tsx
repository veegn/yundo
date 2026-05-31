import { useI18n } from '../context/I18nContext';

export default function Privacy() {
  const { locale } = useI18n();

  return (
    <main className="flex-1 py-12 px-6 max-w-4xl mx-auto">
      <div className="bg-surface-container rounded-3xl p-8 md:p-12 shadow-sm border border-outline-variant/10">
        {locale === 'zh' ? (
          <article className="prose prose-slate max-w-none">
            <h1 className="text-3xl font-bold text-on-surface mb-8">隐私政策</h1>
            <p className="text-on-surface-variant mb-6">最后更新日期：2026年5月31日</p>
            
            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">1. 信息收集</h2>
              <p className="text-on-surface-variant">
                Yundo 是一个自托管的开源项目。在默认运行模式下，我们不会收集、存储或上传您的个人数据到任何中央服务器。所有下载记录、文件缓存和配置信息均存储在您运行该程序的本地设备（SQLite 数据库及本地磁盘）中。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">2. 代理服务与日志</h2>
              <p className="text-on-surface-variant">
                当您使用下载代理或网页代理功能时，程序会向目标服务器发送请求。这些请求可能会包含您的 IP 地址等连接信息，具体取决于您的网络环境和目标服务器的配置。后端程序仅在本地控制台输出必要的运行日志以供调试，不会持久化存储您的访问指纹。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">3. Cookie 与会话</h2>
              <p className="text-on-surface-variant">
                网页代理功能会在您的浏览器中存储必要的会话 Cookie（如 <code>__YUNDO_WEB_SID</code>），用于隔离不同目标网站的访问状态。这些 Cookie 仅用于实现代理功能，且仅存储在您的本地客户端。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">4. 第三方服务</h2>
              <p className="text-on-surface-variant">
                程序集成了 Prometheus 指标导出功能（需手动开启）。如果您开启了此功能并配合第三方监控系统使用，相关运行指标可能会被该系统收集。请自行确保监控系统的安全性。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">5. 安全建议</h2>
              <p className="text-on-surface-variant">
                由于 Yundo 处理网络请求和文件存储，我们强烈建议您将其部署在安全的网络环境中。如果暴露在公网，请务必启用 <code>--api-key</code> 认证功能以保护您的数据安全。
              </p>
            </section>
          </article>
        ) : (
          <article className="prose prose-slate max-w-none">
            <h1 className="text-3xl font-bold text-on-surface mb-8">Privacy Policy</h1>
            <p className="text-on-surface-variant mb-6">Last Updated: May 31, 2026</p>
            
            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">1. Data Collection</h2>
              <p className="text-on-surface-variant">
                Yundo is a self-hosted open-source project. In default operation, we do not collect, store, or upload your personal data to any central server. All download records, file caches, and configurations are stored locally on the device where the software is running (via SQLite and local disk).
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">2. Proxy Services and Logs</h2>
              <p className="text-on-surface-variant">
                When using the download proxy or web proxy features, the program sends requests to target servers. These requests may include connection details such as your IP address, depending on your network environment and the target server's configuration. The backend only outputs necessary runtime logs to the console for debugging and does not persist access fingerprints.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">3. Cookies and Sessions</h2>
              <p className="text-on-surface-variant">
                The web proxy feature stores necessary session cookies in your browser (e.g., <code>__YUNDO_WEB_SID</code>) to isolate access states for different target sites. These cookies are solely used for proxy functionality and reside exclusively on your local client.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">4. Third-party Services</h2>
              <p className="text-on-surface-variant">
                The program includes a Prometheus metrics exporter (optional). If enabled and used with third-party monitoring systems, runtime metrics may be collected by those systems. Please ensure the security of your own monitoring infrastructure.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">5. Security Recommendations</h2>
              <p className="text-on-surface-variant">
                As Yundo handles network requests and file storage, we strongly recommend deploying it in a secure network environment. If exposed to the public internet, ensure that the <code>--api-key</code> authentication is enabled to protect your data.
              </p>
            </section>
          </article>
        )}
      </div>
    </main>
  );
}
