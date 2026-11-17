import type { Metadata } from 'next'
import { ValidatorsView } from '../../src/views/ValidatorsView'

export const metadata: Metadata = { title: 'Validators' }

export default function ValidatorsPage() {
  return <ValidatorsView />
}
