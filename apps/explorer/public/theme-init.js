// Applies the saved light/dark choice before first paint (external file so the CSP needs no inline script).
try {
  var t = localStorage.getItem('xdevs-explorer:theme')
  if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t
} catch (e) {}
