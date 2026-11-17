import Link from 'next/link'

export default function NotFound() {
  return (
    <div className="xu-page w-page">
      <div className="w-empty-account">
        <h1 className="w-soon-title">This page does not exist</h1>
        <p className="w-soon-lede">The wallet has Overview, Send, Receive, Vault, Guardians, Approvals, Names, EOA lock and Settings.</p>
        <div className="xh-ctas">
          <Link className="xh-btn xh-btn-primary xu-btn is-md" href="/">
            Back to the wallet
          </Link>
        </div>
      </div>
    </div>
  )
}
