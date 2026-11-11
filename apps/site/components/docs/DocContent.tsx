import type { ComponentProps, ReactNode } from 'react'
import Link from 'next/link'
import { Fragment, jsx, jsxs } from 'react/jsx-runtime'
import { toJsxRuntime } from 'hast-util-to-jsx-runtime'
import type { Root } from 'hast'
import CopyButton from './CopyButton'
import { Callout, Details } from './Callout'

const LANG_LABEL: Record<string, string> = {
  bash: 'Shell',
  sh: 'Shell',
  shell: 'Shell',
  shellscript: 'Shell',
  ts: 'TypeScript',
  typescript: 'TypeScript',
  js: 'JavaScript',
  json: 'JSON',
  toml: 'TOML',
  solidity: 'Solidity',
  text: 'Text',
}

type DataProps = { 'data-lang'?: string; 'data-callout'?: string; 'data-title'?: string; children?: ReactNode }

function DocLink({ href = '', children, ...rest }: ComponentProps<'a'>) {
  if (href.startsWith('/') && !href.startsWith('//')) {
    return (
      <Link href={href} {...rest}>
        {children}
      </Link>
    )
  }
  return (
    <a href={href} {...rest}>
      {children}
    </a>
  )
}

function CodeBlock(props: ComponentProps<'figure'> & DataProps) {
  const lang = props['data-lang'] ?? 'text'
  return (
    <figure className="d-code" data-lang={lang}>
      <div className="d-code-head">
        <span className="d-code-lang">{LANG_LABEL[lang] ?? lang}</span>
        <CopyButton />
      </div>
      {props.children}
    </figure>
  )
}

function Aside(props: ComponentProps<'aside'> & DataProps) {
  const type = props['data-callout']
  if (!type) return <aside className={props.className}>{props.children}</aside>
  return (
    <Callout type={type} title={props['data-title']}>
      {props.children}
    </Callout>
  )
}

function DetailsBlock(props: ComponentProps<'details'> & DataProps) {
  return <Details title={props['data-title'] || 'Details'}>{props.children}</Details>
}

/** Renders the build-time HAST of a docs page as React server components (no client JS except copy buttons). */
export default function DocContent({ tree }: { tree: Root }) {
  return toJsxRuntime(tree, {
    Fragment,
    jsx,
    jsxs,
    components: {
      a: DocLink,
      figure: CodeBlock,
      aside: Aside,
      details: DetailsBlock,
    } as never,
  })
}
