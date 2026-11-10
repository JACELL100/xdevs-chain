import { useEffect, useState, type FormEvent } from 'react'
import { api, rpc } from './lib/api.ts'
import { config } from './lib/config.ts'
import { classifySearch } from './lib/format.ts'
import { browserStore, recordAddress } from './lib/lookalike.ts'
import { navigate, parseRoute, paths, useLocation } from './lib/router.ts'
import { Link } from './components/ui.tsx'
import { BlocksPage, Home, TxsPage } from './pages/Home.tsx'
import { BlockPage } from './pages/Block.tsx'
import { TxPage } from './pages/Tx.tsx'
import { AddressPage } from './pages/Address.tsx'
import { NamePage, NotFound, SearchPage } from './pages/Misc.tsx'

export function App() {
  const loc = useLocation()
  const [pathname, search = ''] = loc.split(/(?=\?)/)
  const route = parseRoute(pathname ?? '/', search)

  useEffect(() => {
    const titles: Record<string, string> = { home: 'Home', blocks: 'Blocks', txs: 'Transactions', search: 'Search', notfound: 'Not found' }
    const t =
      route.page === 'block' ? `Block ${route.id}` : route.page === 'tx' ? `Tx ${route.hash.slice(0, 10)}…` : route.page === 'address' ? `Address ${route.address.slice(0, 8)}…` : route.page === 'name' ? route.name : titles[route.page]
    document.title = `${t} · xDEVS Explorer`
    document.getElementById('main')?.focus({ preventScroll: true })
  }, [loc])

  let page
  switch (route.page) {
    case 'home':
      page = <Home />
      break
    case 'blocks':
      page = <BlocksPage />
      break
    case 'txs':
      page = <TxsPage />
      break
    case 'block':
      page = <BlockPage key={route.id} id={route.id} />
      break
    case 'tx':
      page = <TxPage key={route.hash} hash={route.hash} />
      break
    case 'address':
      page = <AddressPage key={route.address} address={route.address} tab={route.tab} />
      break
    case 'name':
      page = <NamePage key={route.name} name={route.name} />
      break
    case 'search':
      page = <SearchPage key={route.q} q={route.q} />
      break
    default:
      page = <NotFound />
  }

  return (
    <>
      <a href="#main" className="skip-link">
        Skip to content
      </a>
      <header className="site-header">
        <div className="wrap header-row">
          <Link to={paths.home()} className="brand">
            <img src="/favicon.svg" alt="" width="28" height="28" />
            <span>
              xDEVS <span className="brand-sub">Explorer</span>
            </span>
          </Link>
          <nav aria-label="Main" className="main-nav">
            <Link to={paths.blocks()}>Blocks</Link>
            <Link to={paths.txs()}>Transactions</Link>
          </nav>
          <SearchBar initial={route.page === 'search' ? route.q : ''} />
          <ThemeToggle />
        </div>
      </header>
      <main id="main" tabIndex={-1} className="wrap">
        {page}
      </main>
      <Footer />
    </>
  )
}

function SearchBar({ initial }: { initial: string }) {
  const [q, setQ] = useState(initial)
  useEffect(() => setQ(initial), [initial])
  const submit = (e: FormEvent) => {
    e.preventDefault()
    const t = classifySearch(q)
    if (t.type === 'empty') return
    if (t.type === 'block') navigate(paths.block(t.id))
    else if (t.type === 'address') {
      recordAddress(browserStore(), t.id, 'search', Date.now())
      navigate(paths.address(t.id))
    } else if (t.type === 'name') navigate(paths.name(t.id))
    else navigate(paths.search(t.id))
  }
  return (
    <form role="search" className="search" onSubmit={submit}>
      <label htmlFor="q" className="sr-only">
        Search by block, transaction hash, address or .xdevs name
      </label>
      <input
        id="q"
        type="search"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Block, tx hash, address or name.xdevs"
        autoComplete="off"
        spellCheck={false}
      />
      <button type="submit" className="btn">
        Search
      </button>
    </form>
  )
}

const THEME_KEY = 'xdevs-explorer:theme'
function ThemeToggle() {
  const [theme, setTheme] = useState<string>(() => {
    try {
      return localStorage.getItem(THEME_KEY) ?? 'system'
    } catch {
      return 'system'
    }
  })
  useEffect(() => {
    const root = document.documentElement
    if (theme === 'system') delete root.dataset.theme
    else root.dataset.theme = theme
    try {
      if (theme === 'system') localStorage.removeItem(THEME_KEY)
      else localStorage.setItem(THEME_KEY, theme)
    } catch {
      /* ignore */
    }
  }, [theme])
  const next = theme === 'system' ? 'light' : theme === 'light' ? 'dark' : 'system'
  return (
    <button type="button" className="btn-icon theme" onClick={() => setTheme(next)} aria-label={`Theme: ${theme}. Switch to ${next}.`} title={`Theme: ${theme}`}>
      {theme === 'dark' ? 'Dark' : theme === 'light' ? 'Light' : 'Auto'}
    </button>
  )
}

function Footer() {
  const [net, setNet] = useState<{ chainId: number | null; head: number | null; apiChain: number | null }>({ chainId: null, head: null, apiChain: null })
  useEffect(() => {
    const ctrl = new AbortController()
    void Promise.allSettled([rpc<string>('eth_chainId', [], ctrl.signal), rpc<string>('eth_blockNumber', [], ctrl.signal), api.stats(ctrl.signal)]).then(([c, h, s]) =>
      setNet({
        chainId: c.status === 'fulfilled' ? Number(BigInt(c.value)) : null,
        head: h.status === 'fulfilled' ? Number(BigInt(h.value)) : null,
        apiChain: s.status === 'fulfilled' ? s.value.chainId : null,
      }),
    )
    return () => ctrl.abort()
  }, [])
  const mismatch = (net.chainId !== null && net.chainId !== config.chainId) || (net.apiChain !== null && net.apiChain !== config.chainId)
  return (
    <footer className="site-footer">
      <div className="wrap footer-row">
        <p>
          {config.networkName} · chain id {config.chainId} · RPC {net.head !== null ? `head ${net.head.toLocaleString('en-US')}` : 'unreachable'}
        </p>
        {mismatch ? (
          <p className="danger-text" role="alert">
            Configuration mismatch: RPC chain {String(net.chainId)}, API chain {String(net.apiChain)}, expected {config.chainId}.
          </p>
        ) : null}
        <p className="muted">
          Testnet tokens have no value. Warnings and spam flags are advice, not facts. Built by{' '}
          <a href="https://xdevs.in" rel="noopener noreferrer">
            THE xDEVS
          </a>
          .
        </p>
      </div>
    </footer>
  )
}
