import type { ThemeRegistration } from 'shiki'

/**
 * Code theme tuned to the brand: platinum on near-black, finality teal for
 * strings (the one accent), a quiet warm tone for literals, comments that
 * recede. Background matches --docs-code-bg in components/docs/docs.css.
 */
export const xdevsCodeTheme: ThemeRegistration = {
  name: 'xdevs-dark',
  type: 'dark',
  colors: {
    'editor.background': '#090b10',
    'editor.foreground': '#c9ced6',
  },
  tokenColors: [
    { settings: { foreground: '#c9ced6' } },
    {
      scope: ['comment', 'punctuation.definition.comment', 'string.comment'],
      settings: { foreground: '#5d6470', fontStyle: 'italic' },
    },
    {
      scope: [
        'keyword',
        'storage',
        'storage.type',
        'storage.modifier',
        'keyword.control',
        'keyword.operator.new',
        'keyword.operator.expression',
        'variable.language',
        'support.type.primitive',
      ],
      settings: { foreground: '#f3f5f8' },
    },
    {
      scope: ['keyword.operator', 'punctuation', 'meta.brace', 'punctuation.separator', 'punctuation.terminator'],
      settings: { foreground: '#7d8590' },
    },
    {
      scope: [
        'string',
        'string.quoted',
        'string.template',
        'punctuation.definition.string',
        'markup.inline.raw',
        'string.unquoted',
      ],
      settings: { foreground: '#6fe3d2' },
    },
    {
      scope: [
        'constant.numeric',
        'constant.language',
        'constant.character',
        'constant.other',
        'support.constant',
        'keyword.other.unit',
      ],
      settings: { foreground: '#e6c48f' },
    },
    {
      scope: ['entity.name.function', 'support.function', 'meta.function-call entity.name.function', 'entity.name.command'],
      settings: { foreground: '#ffffff' },
    },
    {
      scope: ['entity.name.type', 'entity.name.class', 'support.type', 'support.class', 'entity.other.inherited-class'],
      settings: { foreground: '#b9c3d6' },
    },
    {
      scope: [
        'support.type.property-name',
        'meta.object-literal.key',
        'variable.other.property',
        'entity.name.tag',
        'meta.mapping.key',
        'support.type.property-name.json',
        'entity.other.attribute-name',
      ],
      settings: { foreground: '#a9b4c4' },
    },
    {
      scope: ['variable.parameter', 'variable.other.readwrite', 'variable', 'meta.definition.variable'],
      settings: { foreground: '#d8dce2' },
    },
    {
      scope: ['entity.name.section', 'markup.heading', 'meta.tag.table', 'entity.name.tag.table.toml'],
      settings: { foreground: '#f3f5f8', fontStyle: 'bold' },
    },
    {
      scope: ['variable.other.normal.shell', 'variable.other.special.shell', 'punctuation.definition.variable.shell'],
      settings: { foreground: '#e6c48f' },
    },
  ],
}
