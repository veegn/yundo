import { useI18n } from '../context/I18nContext';
import { useNavigate } from 'react-router-dom';

export default function Terms() {
  const { locale } = useI18n();
  const navigate = useNavigate();

  return (
    <main className="flex-1 py-12 px-6 max-w-4xl mx-auto w-full relative">
      <button 
        onClick={() => navigate(-1)}
        className="absolute right-10 top-16 p-2 rounded-full hover:bg-surface-container-highest transition-colors text-on-surface-variant z-10"
        title={locale === 'zh' ? '关闭' : 'Close'}
      >
        <span className="material-symbols-outlined">close</span>
      </button>

      <div className="bg-surface-container rounded-3xl p-8 md:p-12 shadow-sm border border-outline-variant/10">
        {locale === 'zh' ? (
          <article className="prose prose-slate max-w-none">
            <h1 className="text-3xl font-bold text-on-surface mb-8">使用条款</h1>
            <p className="text-on-surface-variant mb-6">最后更新日期：2026年5月31日</p>
            
            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">1. 软件许可</h2>
              <p className="text-on-surface-variant">
                Yundo 是一个根据 MIT 许可证发布的开源软件。您可以自由地使用、修改和分发本软件，但需遵守许可证中的相关规定。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">2. 合规使用</h2>
              <p className="text-on-surface-variant">
                您在使用 Yundo 的代理功能、网页浏览或文件上传服务时，必须遵守您所在地区及目标资源托管地区的法律法规。严禁使用本工具从事非法下载、侵犯版权、网络攻击、传播有害信息或其他违反法律的行为。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">3. 免责声明</h2>
              <p className="text-on-surface-variant">
                本软件按“原样”提供，不附带任何形式的明示或暗示担保。作者或版权持有人不对因使用本软件或其代理功能而产生的任何索赔、损害或法律责任（包括但不限于数据丢失、网络封禁或法律诉讼）负责。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">4. 代理与缓存内容</h2>
              <p className="text-on-surface-variant">
                Yundo 仅作为中转工具，其缓存的文件和代理访问的网页内容完全由用户请求的行为决定。用户应对其通过本软件获取和存储的所有内容承担全部责任。
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">5. 修改与终止</h2>
              <p className="text-on-surface-variant">
                作为开源项目，本软件的功能可能会随迭代而发生变更。我们保留在任何时候停止维护或修改特定功能的权利。
              </p>
            </section>
          </article>
        ) : (
          <article className="prose prose-slate max-w-none">
            <h1 className="text-3xl font-bold text-on-surface mb-8">Terms of Use</h1>
            <p className="text-on-surface-variant mb-6">Last Updated: May 31, 2026</p>
            
            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">1. Software License</h2>
              <p className="text-on-surface-variant">
                Yundo is open-source software released under the MIT License. You are free to use, modify, and distribute the software, provided you comply with the terms of the license.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">2. Compliance and Acceptable Use</h2>
              <p className="text-on-surface-variant">
                When using Yundo's proxy, web browsing, or file upload services, you must comply with the laws of your local jurisdiction and the jurisdiction where target resources are hosted. Using this tool for illegal downloads, copyright infringement, cyberattacks, spreading harmful information, or any other illegal activities is strictly prohibited.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">3. Disclaimer of Liability</h2>
              <p className="text-on-surface-variant">
                THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND. In no event shall the authors or copyright holders be liable for any claim, damages, or other liability (including but not limited to data loss, network bans, or legal action) arising from the use of this software or its proxy functions.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">4. Proxy and Cached Content</h2>
              <p className="text-on-surface-variant">
                Yundo acts solely as a transit tool. The cached files and proxied web content are determined entirely by user requests. Users assume full responsibility for all content accessed or stored via the software.
              </p>
            </section>

            <section className="mb-8">
              <h2 className="text-xl font-bold text-on-surface mb-4">5. Modifications and Termination</h2>
              <p className="text-on-surface-variant">
                As an open-source project, features may change over time. we reserve the right to modify or discontinue specific features at any time.
              </p>
            </section>
          </article>
        )}
      </div>
    </main>
  );
}
