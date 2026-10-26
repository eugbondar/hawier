const treeKeyPrefix = 'hawier:tree:'

function selectionBoxes() {
  return Array.from(document.querySelectorAll('input[name="ids"]'))
}

function selectedBoxes() {
  return selectionBoxes().filter((box) => box.checked)
}

function initClickableRows() {
  document.addEventListener('click', (event) => {
    const target = event.target
    if (!(target instanceof HTMLElement)) return
    const row = target.closest('tr[data-href]')
    if (!row) return
    // The whole selection cell is a label: clicking anywhere in it — even a
    // pixel beside the box — must select, never navigate.
    if (target.closest('a, button, input, label, textarea, select, .select-cell')) return
    // Ctrl/Cmd or Shift extend the selection instead of following the row.
    const box = row.querySelector('input[name="ids"]')
    if (box && (event.ctrlKey || event.metaKey || event.shiftKey)) {
      event.preventDefault()
      applySelectionClick(box, event.shiftKey)
      return
    }
    window.location.href = row.dataset.href || window.location.href
  })
}

function initTreeState() {
  document.querySelectorAll('details[data-tree-key]').forEach((node) => {
    const details = node
    const key = treeKeyPrefix + details.dataset.treeKey
    const stored = window.localStorage.getItem(key)
    if (stored === 'open') details.open = true
    if (stored === 'closed' && !details.dataset.active) details.open = false
    details.addEventListener('toggle', () => {
      window.localStorage.setItem(key, details.open ? 'open' : 'closed')
    })
  })
}

function initSearchShortcut() {
  document.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
      const search = document.querySelector('.global-search input[name="q"]')
      if (search) {
        event.preventDefault()
        search.focus()
        search.select()
      }
    }
  })
}

function initFormHelpers() {
  document.addEventListener('submit', (event) => {
    const form = event.target
    if (!(form instanceof HTMLFormElement)) return
    // A destructive bulk operation must say how many rows it will hit. The
    // confirmation lives on the clicked button, so read it from the
    // submitter — reading it off the form silently skipped it before.
    const submitter = event.submitter
    const bulkDanger = submitter && submitter.dataset ? submitter.dataset.bulkDanger : null
    if (bulkDanger) {
      const count = selectedBoxes().length
      if (!window.confirm(`${bulkDanger} ${count} row(s)? This cannot be undone.`)) {
        event.preventDefault()
        return
      }
    }
    const message = form.dataset.confirm
    if (message && !window.confirm(message)) {
      event.preventDefault()
      return
    }
    const columnsInput = form.querySelector('[data-columns-value]')
    if (columnsInput) {
      const checkedColumns = Array.from(form.querySelectorAll('input[name="col"]:checked')).map(
        (input) => input.value,
      )
      columnsInput.value = checkedColumns.join(',')
    }
  })
}

function initLiveSearch() {
  const form = document.querySelector('.global-search')
  const input = form?.querySelector('input[name="q"]')
  const panel = form?.querySelector('[data-live-search-results]')
  if (!form || !input || !panel) return

  let timer = 0
  let controller = null

  const load = async (q) => {
    controller?.abort()
    controller = new AbortController()
    try {
      const response = await fetch(`/search/live?q=${encodeURIComponent(q)}`, {
        headers: { Accept: 'text/html' },
        signal: controller.signal,
      })
      panel.innerHTML = await response.text()
      formatTimes(panel)
    } catch (error) {
      if (error.name !== 'AbortError') {
        panel.innerHTML = '<div class="live-search-state danger">Search failed</div>'
      }
    }
  }

  input.addEventListener('input', () => {
    window.clearTimeout(timer)
    const q = input.value.trim()
    if (q.length === 0) {
      // Empty box: show this user's recent searches instead of nothing.
      panel.hidden = false
      timer = window.setTimeout(() => load(''), 120)
      return
    }
    if (q.length < 2) {
      panel.hidden = true
      panel.innerHTML = ''
      return
    }
    panel.hidden = false
    panel.innerHTML = '<div class="live-search-state">Searching...</div>'
    timer = window.setTimeout(() => load(q), 180)
  })

  input.addEventListener('focus', () => {
    if (input.value.trim().length === 0) {
      panel.hidden = false
      load('')
    }
  })

  input.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') panel.hidden = true
  })

  document.addEventListener('click', (event) => {
    if (!form.contains(event.target)) panel.hidden = true
  })
}

function initAutoRefresh() {
  // Pages carrying data-autorefresh="<seconds>" (rollout detail, action
  // history) reload themselves, paused while a form field has focus or a
  // selection is in progress.
  const refresher = document.querySelector('[data-autorefresh]')
  if (!refresher) return
  const seconds = parseInt(refresher.getAttribute('data-autorefresh'), 10) || 5
  window.setInterval(() => {
    const active = document.activeElement
    const busy = active && ['INPUT', 'TEXTAREA', 'SELECT'].includes(active.tagName)
    if (!busy && selectedBoxes().length === 0) window.location.reload()
  }, seconds * 1000)
}

function initCopyButtons() {
  document.addEventListener('click', (event) => {
    const button = event.target.closest('[data-copy]')
    if (!button) return
    const original = button.textContent
    navigator.clipboard
      .writeText(button.getAttribute('data-copy'))
      .then(() => {
        button.textContent = 'copied'
      })
      .catch(() => {
        button.textContent = 'copy failed'
      })
      .finally(() => {
        window.setTimeout(() => {
          button.textContent = original
        }, 1200)
      })
  })
}

function initTokenReveal() {
  document.addEventListener('click', (event) => {
    const button = event.target.closest('[data-reveal-token]')
    if (!button) return
    const code = button.parentElement.querySelector('code[data-token]')
    if (!code) return
    if (code.textContent.indexOf('•') !== -1) {
      code.textContent = code.getAttribute('data-token')
      button.textContent = 'Hide'
    } else {
      code.textContent = '•'.repeat(8)
      button.textContent = 'Show'
    }
  })
}

/// Server-rendered timestamps carry epoch MILLISECONDS and read as UTC;
/// upgrade them to the reader's own timezone, keeping UTC in the tooltip.
function formatTimes(root) {
  const pad = (value) => String(value).padStart(2, '0')
  ;(root || document).querySelectorAll('time[data-epoch]').forEach((el) => {
    const epoch = Number.parseInt(el.getAttribute('data-epoch'), 10)
    if (!Number.isFinite(epoch)) return
    const at = new Date(epoch)
    const time = [at.getHours(), at.getMinutes(), at.getSeconds()].map(pad).join(':')
    const date = [at.getDate(), at.getMonth() + 1, at.getFullYear() % 100].map(pad).join('.')
    el.textContent = time + ' ' + date
    el.dataset.localized = 'true'
  })
}

function initTimes() {
  formatTimes(document)
}

let selectionAnchor = null

/// Give every row between the anchor and `box` the state `box` now has.
function extendSelection(box) {
  const list = selectionBoxes()
  const from = list.indexOf(selectionAnchor)
  const to = list.indexOf(box)
  if (from !== -1 && to !== -1) {
    const [start, end] = from < to ? [from, to] : [to, from]
    for (let i = start; i <= end; i += 1) list[i].checked = box.checked
  }
  selectionAnchor = box
  refreshSelection()
}

/// Ctrl/Cmd- or Shift-click on the row body (not on the box itself).
function applySelectionClick(box, extend) {
  if (extend && selectionAnchor !== null) {
    box.checked = true
    extendSelection(box)
    return
  }
  box.checked = !box.checked
  selectionAnchor = box
  refreshSelection()
}

/// Selected rows are highlighted, counted, and offered a menu of operations.
function refreshSelection() {
  const selected = selectedBoxes()
  const all = document.querySelector('[data-select-all]')
  for (const box of selectionBoxes()) {
    const row = box.closest('tr')
    if (row) row.classList.toggle('selected', box.checked)
  }
  for (const counter of document.querySelectorAll('[data-bulk-count]')) {
    counter.textContent = `${selected.length} selected`
  }
  const trigger = document.querySelector('[data-bulk-menu]')
  if (trigger) trigger.hidden = selected.length === 0
  if (all) {
    const total = selectionBoxes().length
    all.checked = selected.length > 0 && selected.length === total
    all.indeterminate = selected.length > 0 && selected.length < total
  }
  if (selected.length === 0) closeContextMenu()
}

function closeContextMenu() {
  const menu = document.querySelector('[data-context-menu]')
  if (menu) menu.hidden = true
}

function openContextMenu(x, y) {
  const menu = document.querySelector('[data-context-menu]')
  if (!menu || selectedBoxes().length === 0) return
  menu.hidden = false
  // Keep the menu inside the viewport whichever corner it was opened in.
  const rect = menu.getBoundingClientRect()
  const left = Math.min(x, window.innerWidth - rect.width - 8)
  const top = Math.min(y, window.innerHeight - rect.height - 8)
  menu.style.left = `${Math.max(8, left)}px`
  menu.style.top = `${Math.max(8, top)}px`
}

function initSelection() {
  const table = document.querySelector('table[data-selectable]')
  if (!table) return

  document.addEventListener('change', (event) => {
    const all = event.target.closest('[data-select-all]')
    if (all) {
      for (const box of selectionBoxes()) box.checked = all.checked
      refreshSelection()
      return
    }
    if (event.target.closest('input[name="ids"]')) refreshSelection()
  })

  // Shift-click inside the selection cell extends the range. The browser
  // has already flipped this box by the time the click handler runs.
  document.addEventListener('click', (event) => {
    const box = event.target.closest('input[name="ids"]')
    if (!box) return
    if (event.shiftKey && selectionAnchor !== null) {
      extendSelection(box)
      return
    }
    selectionAnchor = box
  })

  table.addEventListener('contextmenu', (event) => {
    const row = event.target.closest('tr[data-href]')
    if (!row) return
    const box = row.querySelector('input[name="ids"]')
    // Right-clicking an unselected row selects it first, like a file manager.
    if (box && !box.checked) {
      for (const other of selectionBoxes()) other.checked = false
      box.checked = true
      selectionAnchor = box
      refreshSelection()
    }
    if (selectedBoxes().length === 0) return
    event.preventDefault()
    openContextMenu(event.clientX, event.clientY)
  })

  document.addEventListener('click', (event) => {
    if (event.target.closest('[data-bulk-menu]')) {
      const rect = event.target.closest('[data-bulk-menu]').getBoundingClientRect()
      openContextMenu(rect.left, rect.bottom + 2)
      return
    }
    if (!event.target.closest('[data-context-menu]')) closeContextMenu()
  })

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') closeContextMenu()
  })

  refreshSelection()
}

/// Typeahead field: no dropdown is ever filled with a whole server list.
function initPickers() {
  for (const picker of document.querySelectorAll('[data-picker]')) {
    const input = picker.querySelector('[data-picker-input]')
    const hidden = picker.querySelector('[data-picker-value]')
    const results = picker.querySelector('[data-picker-results]')
    if (!input || !hidden || !results) continue
    const resource = picker.dataset.pickerResource
    // A free-text picker suggests what already exists but accepts anything
    // typed: a target group is a path the operator may be inventing now.
    // Without this the value would be cleared on every keystroke and the
    // form would post an empty field while looking like it worked.
    const free = picker.hasAttribute('data-picker-free')
    let timer = 0
    let controller = null

    const close = () => {
      results.hidden = true
    }

    const load = async (q) => {
      controller?.abort()
      controller = new AbortController()
      try {
        const response = await fetch(
          `/pick/${encodeURIComponent(resource)}?q=${encodeURIComponent(q)}`,
          { headers: { Accept: 'text/html' }, signal: controller.signal },
        )
        results.innerHTML = await response.text()
        results.hidden = false
      } catch (error) {
        if (error.name !== 'AbortError') {
          results.innerHTML = '<span class="picker-state">Lookup failed</span>'
          results.hidden = false
        }
      }
    }

    input.addEventListener('input', () => {
      // Typing invalidates the previous choice: the id must match the text.
      // Unless the field is free text, where the text *is* the value.
      hidden.value = free ? input.value.trim() : ''
      window.clearTimeout(timer)
      timer = window.setTimeout(() => load(input.value.trim()), 180)
    })

    input.addEventListener('focus', () => load(input.value.trim()))

    input.addEventListener('keydown', (event) => {
      const options = Array.from(results.querySelectorAll('.picker-option'))
      if (event.key === 'Escape') {
        close()
        return
      }
      if (event.key === 'Enter' && !results.hidden && options.length > 0) {
        event.preventDefault()
        const active = results.querySelector('.picker-option.active') || options[0]
        active.click()
        return
      }
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
      event.preventDefault()
      if (options.length === 0) return
      const current = options.findIndex((option) => option.classList.contains('active'))
      const step = event.key === 'ArrowDown' ? 1 : -1
      const next = (current + step + options.length) % options.length
      options.forEach((option, index) => option.classList.toggle('active', index === next))
      options[next].scrollIntoView({ block: 'nearest' })
    })

    results.addEventListener('click', (event) => {
      const option = event.target.closest('.picker-option')
      if (!option) return
      hidden.value = option.dataset.value || ''
      input.value = option.dataset.label || ''
      close()
    })

    document.addEventListener('click', (event) => {
      if (!picker.contains(event.target)) close()
    })
  }
}

/// The filter sidebar is off by default — 99% of visits never need it.
// A cell the CSS had to clip keeps its value in a tooltip. Only the clipped
// ones: a tooltip repeating text you can already read is noise, and the
// hover would fire on every cell in the table.
function initCellTitles() {
  for (const cell of document.querySelectorAll('td')) {
    if (cell.title) continue
    if (cell.scrollWidth > cell.clientWidth) {
      cell.title = cell.textContent.trim()
    }
  }
}

function initFilterToggle() {
  const key = 'hawier:filters'
  if (window.localStorage.getItem(key) === 'open') {
    document.body.classList.add('filters-open')
  }
  document.addEventListener('click', (event) => {
    if (!event.target.closest('[data-filters-toggle]')) return
    const open = document.body.classList.toggle('filters-open')
    window.localStorage.setItem(key, open ? 'open' : 'closed')
  })
}

/// Object pages can show values exactly as the API returned them.
function initRawToggle() {
  const key = 'hawier:raw'
  const toggle = document.querySelector('[data-raw-toggle]')
  const apply = (raw) => document.body.classList.toggle('raw-values', raw)
  apply(window.localStorage.getItem(key) === 'raw')
  if (!toggle) return
  toggle.checked = document.body.classList.contains('raw-values')
  toggle.addEventListener('change', () => {
    apply(toggle.checked)
    window.localStorage.setItem(key, toggle.checked ? 'raw' : 'cooked')
  })
}

function initSidebarToggle() {
  const key = 'hawier:sidebar'
  if (window.localStorage.getItem(key) === 'collapsed') {
    document.body.classList.add('sidebar-collapsed')
  }
  const toggle = document.querySelector('[data-sidebar-toggle]')
  if (!toggle) return
  toggle.addEventListener('click', () => {
    const collapsed = document.body.classList.toggle('sidebar-collapsed')
    window.localStorage.setItem(key, collapsed ? 'collapsed' : 'expanded')
  })
}

/* What's This? — arm from the top bar or Shift+F1, then click anything and the
   panel answers for that control instead of the control acting. */
function initHelpMode() {
  const panel = document.querySelector('[data-help-panel]')
  if (!panel) return
  const toggle = document.querySelector('[data-help-toggle]')
  let armed = false

  const arm = (on) => {
    armed = on
    document.body.classList.toggle('help-mode', on)
    if (toggle) toggle.setAttribute('aria-pressed', on ? 'true' : 'false')
  }
  const close = () => {
    panel.hidden = true
    panel.innerHTML = ''
  }
  const open = async (key) => {
    try {
      const url = '/help/panel?key=' + encodeURIComponent(key)
      const response = await fetch(url, { credentials: 'same-origin' })
      if (!response.ok) return
      panel.innerHTML = await response.text()
      panel.hidden = false
    } catch (error) {
      console.error('hawier: help panel failed', error)
    }
  }
  const answer = (event) => {
    const target = event.target
    const holder = target.closest ? target.closest('[data-help]') : null
    const key = holder ? holder.getAttribute('data-help') : document.body.dataset.help
    if (!event.ctrlKey && !event.metaKey) arm(false)
    open(key || 'console/help#the-manual')
  }

  if (toggle) {
    toggle.addEventListener('click', (event) => {
      event.preventDefault()
      arm(!armed)
    })
  }
  panel.addEventListener('click', (event) => {
    if (event.target.closest('[data-help-close]')) close()
  })
  document.addEventListener('keydown', (event) => {
    if (event.key === 'F1' && event.shiftKey) {
      event.preventDefault()
      arm(!armed)
      return
    }
    if (event.key !== 'Escape') return
    if (armed) arm(false)
    else if (!panel.hidden) close()
  })

  /* Capture phase, and mousedown and submit as well as click: clickable rows,
     the picker, selection and the form helpers all listen while the event
     bubbles, and a bubbling listener cannot stop a sibling that already ran.
     Enter inside a text field never produces a click at all. */
  const intercept = (event) => {
    if (!armed) return
    if (panel.contains(event.target)) return
    event.preventDefault()
    event.stopPropagation()
    if (toggle && toggle.contains(event.target)) {
      if (event.type === 'click') arm(false)
      return
    }
    if (event.type === 'click') answer(event)
  }
  document.addEventListener('mousedown', intercept, true)
  document.addEventListener('click', intercept, true)
  document.addEventListener('submit', intercept, true)

  document.addEventListener('click', (event) => {
    if (armed || panel.hidden) return
    if (panel.contains(event.target)) return
    close()
  })
}

// One broken initialiser must never take the rest of the page with it:
// a missing semicolon here once killed copy, token reveal, auto-refresh,
// time formatting and the sidebar toggle in one go.
for (const init of [
  initClickableRows,
  initTreeState,
  initSearchShortcut,
  initFormHelpers,
  initLiveSearch,
  initAutoRefresh,
  initCopyButtons,
  initTokenReveal,
  initTimes,
  initSidebarToggle,
  initSelection,
  initPickers,
  initFilterToggle,
  initRawToggle,
  initCellTitles,
  initHelpMode,
]) {
  try {
    init()
  } catch (error) {
    console.error(`hawier: ${init.name} failed`, error)
  }
}
