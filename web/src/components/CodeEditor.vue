<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { EditorState } from '@codemirror/state'
import { EditorView, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from '@codemirror/view'
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
import { searchKeymap, highlightSelectionMatches } from '@codemirror/search'
import { defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language'
import { oneDark } from '@codemirror/theme-one-dark'
import { javascript } from '@codemirror/lang-javascript'
import { json } from '@codemirror/lang-json'
import { yaml } from '@codemirror/lang-yaml'
import { python } from '@codemirror/lang-python'
import { rust } from '@codemirror/lang-rust'
import { html } from '@codemirror/lang-html'
import { css } from '@codemirror/lang-css'
import { sql } from '@codemirror/lang-sql'
import { markdown } from '@codemirror/lang-markdown'

const props = defineProps<{
  modelValue: string
  filename: string
  dark: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', val: string): void
  (e: 'save'): void
}>()

const hostEl = ref<HTMLDivElement | null>(null)
let view: EditorView | null = null

function pickLanguage(filename: string) {
  const lower = filename.toLowerCase()
  if (lower.endsWith('.json')) return json()
  if (lower.endsWith('.yml') || lower.endsWith('.yaml')) return yaml()
  if (lower.endsWith('.py')) return python()
  if (lower.endsWith('.rs')) return rust()
  if (lower.endsWith('.html') || lower.endsWith('.htm') || lower.endsWith('.vue')) return html()
  if (lower.endsWith('.css') || lower.endsWith('.scss')) return css()
  if (lower.endsWith('.sql')) return sql()
  if (lower.endsWith('.md')) return markdown()
  if (lower.endsWith('.js') || lower.endsWith('.ts') || lower.endsWith('.mjs') || lower.endsWith('.tsx')) {
    return javascript({ typescript: lower.endsWith('.ts') || lower.endsWith('.tsx') })
  }
  return []
}

function createEditor() {
  if (!hostEl.value) return
  view?.destroy()

  const extensions = [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightActiveLine(),
    history(),
    highlightSelectionMatches(),
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
    pickLanguage(props.filename),
    keymap.of([
      {
        key: 'Mod-s',
        preventDefault: true,
        run: () => {
          emit('save')
          return true
        },
      },
      indentWithTab,
      ...defaultKeymap,
      ...historyKeymap,
      ...searchKeymap,
    ]),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        emit('update:modelValue', update.state.doc.toString())
      }
    }),
    EditorView.theme({
      '&': { height: '100%', fontSize: '13px' },
      '.cm-scroller': {
        overflow: 'auto',
        fontFamily: '"JetBrains Mono", "Fira Code", Consolas, monospace',
      },
    }),
  ]

  if (props.dark) {
    extensions.push(oneDark)
  }

  const state = EditorState.create({
    doc: props.modelValue,
    extensions,
  })
  view = new EditorView({ state, parent: hostEl.value })
}

watch(
  () => [props.filename, props.dark],
  () => createEditor(),
)

watch(
  () => props.modelValue,
  (val) => {
    if (!view) return
    const cur = view.state.doc.toString()
    if (cur !== val) {
      view.dispatch({
        changes: { from: 0, to: cur.length, insert: val },
      })
    }
  },
)

onMounted(() => createEditor())
onBeforeUnmount(() => view?.destroy())
</script>

<template>
  <div ref="hostEl" class="h-full w-full overflow-hidden rounded-lg border" style="border-color: var(--zm-border)" />
</template>
