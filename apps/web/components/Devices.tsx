/** Device frames drawn in CSS (see .phone / .laptop / .browser in globals.css). */
import type { ReactNode } from 'react'

export function Phone({ src, alt, children, eager = false }: { src?: string; alt?: string; children?: ReactNode; eager?: boolean }) {
  return (
    <div className="phone">
      <div className="screen">
        <div className="island" aria-hidden="true" />
        <div className="statusbar" aria-hidden="true">
          <span>9:41</span>
          <i>
            <b />
          </i>
        </div>
        {src ? (
          <img src={src} alt={alt ?? ''} width={640} height={1385} loading={eager ? 'eager' : 'lazy'} decoding="async" />
        ) : null}
        {children}
      </div>
    </div>
  )
}

export function Laptop({ src, alt, eager = false }: { src: string; alt: string; eager?: boolean }) {
  const small = src.replace(/\.webp$/, '-800.webp')
  return (
    <div className="laptop">
      <div className="lid">
        <div className="display">
          <img
            src={src}
            srcSet={`${small} 800w, ${src} 1440w`}
            sizes="(max-width: 900px) 92vw, 720px"
            alt={alt}
            width={1440}
            height={900}
            loading={eager ? 'eager' : 'lazy'}
            decoding="async"
          />
        </div>
      </div>
      <div className="base" aria-hidden="true" />
    </div>
  )
}

export function Browser({ src, alt, url }: { src: string; alt: string; url: string }) {
  const small = src.replace(/\.webp$/, '-800.webp')
  return (
    <div className="browser">
      <div className="bar" aria-hidden="true">
        <span className="lights">
          <i />
          <i />
          <i />
        </span>
        <span className="url">{url}</span>
      </div>
      <img
        src={src}
        srcSet={`${small} 800w, ${src} 1440w`}
        sizes="(max-width: 900px) 92vw, 720px"
        alt={alt}
        width={1440}
        height={900}
        loading="lazy"
        decoding="async"
      />
    </div>
  )
}
