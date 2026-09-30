/** @type {import('tailwindcss').Config} */
export default {
  darkMode: ['class', '[data-theme="dark"]'],
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: 'rgb(var(--bg) / <alpha-value>)',
        surface: 'rgb(var(--surface) / <alpha-value>)',
        surface2: 'rgb(var(--surface-2) / <alpha-value>)',
        line: 'rgb(var(--line) / <alpha-value>)',
        ink: 'rgb(var(--text) / <alpha-value>)',
        ink2: 'rgb(var(--text-2) / <alpha-value>)',
        ink3: 'rgb(var(--text-3) / <alpha-value>)',
        accent: 'rgb(var(--accent) / <alpha-value>)',
        ok: 'rgb(var(--ok) / <alpha-value>)',
        warn: 'rgb(var(--warn) / <alpha-value>)',
        danger: 'rgb(var(--danger) / <alpha-value>)',
        nr: 'rgb(var(--nr) / <alpha-value>)',
        onaccent: 'rgb(var(--on-accent) / <alpha-value>)',
        scrim: 'rgb(var(--scrim) / <alpha-value>)',
        band: 'rgb(var(--band) / <alpha-value>)',
        bandink: 'rgb(var(--band-ink) / <alpha-value>)',
        bandink2: 'rgb(var(--band-ink-2) / <alpha-value>)',
        bandline: 'rgb(var(--band-line) / <alpha-value>)',
        bandaccent: 'rgb(var(--band-accent) / <alpha-value>)',
      },
      fontFamily: {
        sans: ['system-ui', '-apple-system', "'Segoe UI'", 'Roboto', 'sans-serif'],
        display: ["'Space Grotesk'", 'system-ui', 'sans-serif'],
        mono: ["'JetBrains Mono'", 'ui-monospace', 'SFMono-Regular', 'Menlo', 'monospace'],
      },
      // Tailwind 3 only generates opacity modifiers on this scale; the default
      // scale lacks these steps, so e.g. `border-line/8` silently emitted nothing.
      opacity: {
        4: '0.04',
        6: '0.06',
        8: '0.08',
        12: '0.12',
      },
      fontSize: {
        caption: ['0.6875rem', { lineHeight: '1rem' }],
        meta: ['0.75rem', { lineHeight: '1.125rem' }],
        body: ['0.8125rem', { lineHeight: '1.25rem' }],
      },
      borderRadius: {
        panel: '10px',
        ctl: '6px',
        chip: '4px',
      },
      maxWidth: {
        shell: '72rem',
      },
    },
  },
  plugins: [],
}
