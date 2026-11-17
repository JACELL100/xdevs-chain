import { EmptyState } from '@xdevs/ui'

export default function NotFound() {
  return (
    <div className="xu-page x-page">
      <EmptyState
        title="Page not found"
        description="That page does not exist. Search for a block, transaction, address or .xdevs name instead."
        action={
          <a className="xh-btn xh-btn-ghost xu-btn is-sm" href="/">
            Back to the overview
          </a>
        }
      />
    </div>
  )
}
