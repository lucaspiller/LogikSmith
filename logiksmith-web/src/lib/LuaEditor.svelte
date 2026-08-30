<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
  import { StreamLanguage, syntaxHighlighting, defaultHighlightStyle, bracketMatching, indentUnit } from '@codemirror/language';
  import { lua } from '@codemirror/legacy-modes/mode/lua';
  import { lintGutter, setDiagnostics, type Diagnostic } from '@codemirror/lint';
  import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
  import { vim } from '@replit/codemirror-vim';
  import { Compartment, EditorState } from '@codemirror/state';
  import { EditorView, drawSelection, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers, placeholder } from '@codemirror/view';
  import { createEventDispatcher } from 'svelte';
  import type { DraftValidationError } from './block-workbench';

  export let source = '';
  export let label = 'Lua source';
  export let diagnostics: DraftValidationError[] = [];
  export let disabled = false;

  const dispatch = createEventDispatcher<{ change: string; save: void }>();
  let host: HTMLDivElement;
  let view: EditorView | null = null;
  let lastSource = source;
  let pendingLocalSource: string | null = null;
  let syncingSource = false;
  const editableCompartment = new Compartment();
  const vimCompartment = new Compartment();
  let vimMode = false;

  function toggleVimMode(): void {
    vimMode = !vimMode;
    view?.dispatch({ effects: vimCompartment.reconfigure(vimMode ? vim() : []) });
  }

  function diagnosticRange(error: DraftValidationError): Diagnostic | null {
    if (error.line === null || !view) return null;
    const line = view.state.doc.line(Math.min(error.line, view.state.doc.lines));
    return { from: line.from, to: Math.max(line.from, line.to), severity: 'error', message: `${error.category}: ${error.message}` };
  }

  function updateDiagnostics(): void {
    if (!view) return;
    const next = diagnostics.map(diagnosticRange).filter((item): item is Diagnostic => item !== null);
    view.dispatch(setDiagnostics(view.state, next));
  }

  onMount(() => {
    const update = EditorView.updateListener.of((event) => {
      if (!event.docChanged || syncingSource) return;
      const next = event.state.doc.toString();
      pendingLocalSource = next;
      dispatch('change', next);
    });
    const state = EditorState.create({
      doc: source,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        bracketMatching(),
        history(),
        highlightSelectionMatches(),
        lintGutter(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        StreamLanguage.define(lua),
        indentUnit.of('  '),
        keymap.of([{ key: 'Mod-s', run: () => { dispatch('save'); return true; } }, ...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        placeholder('function handle(event, input, meta, state, ctx) …'),
        EditorView.lineWrapping,
        editableCompartment.of(EditorView.editable.of(!disabled)),
        vimCompartment.of([]),
        update,
        EditorView.theme({
          '&': { minHeight: '300px', maxHeight: '620px', border: '1px solid #9ba8b1', borderRadius: '3px', backgroundColor: '#f8fafb', fontSize: '0.82rem' },
          '.cm-scroller': { overflow: 'auto', fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace' },
          '.cm-content': { padding: '12px 0' },
          '.cm-gutters': { backgroundColor: '#e7edf1', color: '#607080', border: '0' },
          '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: '#e8f0f4' },
          '.cm-diagnostic-error': { borderLeft: '3px solid #b42318' },
          '.cm-lintRange-error': { backgroundImage: 'none', textDecoration: 'underline wavy #b42318' }
        })
      ]
    });
    view = new EditorView({ state, parent: host });
    host.setAttribute('aria-label', label);
    updateDiagnostics();
  });

  function syncSource(): void {
    if (!view) return;
    const current = view.state.doc.toString();
    if (pendingLocalSource !== null) {
      if (source === pendingLocalSource) {
        pendingLocalSource = null;
        lastSource = source;
        return;
      }
      if (current === pendingLocalSource) return;
      pendingLocalSource = null;
    }
    if (source === current) {
      lastSource = source;
      return;
    }
    lastSource = source;
    syncingSource = true;
    try {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
    } finally {
      syncingSource = false;
    }
  }

  $: if (view && (source !== lastSource || pendingLocalSource !== null)) syncSource();
  $: if (view) updateDiagnostics();
  $: if (view) view.dispatch({ effects: editableCompartment.reconfigure(EditorView.editable.of(!disabled)) });

  onDestroy(() => view?.destroy());
</script>

<div class="lua-editor-toolbar">
  <span class="subtle">Editing mode: {vimMode ? 'Vim' : 'Standard'}</span>
  <button type="button" class="small-button" disabled={disabled} aria-pressed={vimMode} on:click={toggleVimMode}>{vimMode ? 'Use standard mode' : 'Enable Vim mode'}</button>
</div>
<div class="lua-editor" bind:this={host} role="textbox" aria-label={label} aria-multiline="true" aria-describedby={`${label.replaceAll(' ', '-').toLowerCase()}-help`}></div>
<p id={`${label.replaceAll(' ', '-').toLowerCase()}-help`} class="subtle">Lua editor with line numbers, bracket matching, find/replace, undo/redo, keyboard indentation, and optional Vim bindings.</p>
