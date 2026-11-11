import type { ReactNode } from 'react'
import Header from '@/components/site/Header'
import Footer from '@/components/site/Footer'
import DocsShell, { MenuButton } from '@/components/docs/DocsShell'
import Sidebar from '@/components/docs/Sidebar'
import Search from '@/components/docs/Search'
import '@/components/docs/docs.css'

export default function DocsLayout({ children }: { children: ReactNode }) {
  return (
    <DocsShell>
      <Header variant="docs" search={<Search />} mobileStart={<MenuButton />} />
      <div className="d-shell">
        <Sidebar />
        <main id="main" className="d-main" tabIndex={-1}>
          {children}
        </main>
      </div>
      <div className="d-footer">
        <Footer variant="compact" />
      </div>
    </DocsShell>
  )
}
