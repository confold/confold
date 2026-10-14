# Confold Desktop Design System

## 1. Atmosphere & Identity

Confold is a focused desktop utility: quiet, keyboard-friendly, and precise. Keep the existing native-feeling modal and form language; the source picker screenshot is the fidelity reference for this change. Source identity comes from provider marks where available, with glyphs only as a fallback for existing providers.

## 2. Color

| Role | Light | Dark | Usage |
|---|---|---|---|
| Canvas | `#f6f6f6` | `#1e1e1e` | Main application |
| Surface | `#ffffff` | `#2a2a2a` | Dialogs and form surfaces |
| Surface/input | `#ffffff` | `#1e1e1e` | Text inputs |
| Text primary | `#1a1a1a` | `#f0f0f0` | Main copy |
| Text muted | `#6b7280` | `#a0a0a0` | Hints and secondary labels |
| Border | `#cccccc` | `#444444` | Inputs and control outlines |
| Accent | `#396cd8` | `#396cd8` | Primary actions and keyboard focus |
| Success | `#2ea043` | `#2ea043` | Valid source state |
| Error | `#e5484d` | `#e5484d` | Inline failures |

Provider brand colors belong only inside their brand marks; do not introduce them as UI accents.

## 3. Typography

- Primary: `Inter, Avenir, Helvetica, Arial, sans-serif` (existing app stack).
- Monospace: `ui-monospace, monospace` for paths and technical identifiers.
- Modal title: approximately 1.1rem, semibold/bold.
- Field labels: approximately 0.8rem, semibold; body/helper text: 0.82–0.9rem.
- Preserve existing scale; no new type family for the route browser.

## 4. Spacing & Layout

- Base rhythm: 4px; existing form/modal gaps use 0.2–0.8rem and modal padding 1.2rem.
- Source picker remains a bounded modal (`min(540px, 92vw)`) with one vertical scroll owner: the modal body.
- The route browser replaces the form body in-place, not as a nested modal; header/back, breadcrumb, search, results and actions remain in that scroll region.
- At narrow widths, controls wrap or stack; route path text truncates/wraps and never forces horizontal scrolling.

## 5. Components

### Source type button
- Structure: semantic button, provider mark, provider name.
- States: default, hover, selected, focus-visible, disabled.
- Selected state uses the existing accent fill; focus-visible remains visibly keyboard accessible.

### Source form field
- Structure: label above control; required marker and secret indicator stay adjacent to label.
- States: default, missing, disabled, focus-visible, connection error.
- Path fields may expose an adjacent browse action; label remains the accessible name.

### Nuxeo route browser
- Structure: back action, current-path breadcrumb, labeled search input, folder result list, load-more state, select-current-folder action.
- States: loading, results, empty, error, more-results, selected path.
- Search results and folder entries are semantic buttons; keyboard navigation and focus remain in document order.
- One scroll owner: results region inside the bounded modal body. Reduced-motion behavior is static; no decorative motion.
- Bound the result list to `min(34vh, 18rem)` so large directories scroll independently without displacing modal actions.

### Source icon
- Provider logo is an image with an accessible empty alt when the adjacent provider name labels it; constrain it to the existing icon slot and preserve aspect ratio.
- Nuxeo's horizontal wordmark is 1.05em high and capped at 5.6em in the type picker; compact source summaries use a 0.9em/3.8em cap so path labels remain readable.
- Existing provider glyphs remain fallback until their marks are migrated.

## 6. Motion & Interaction

- Existing modal enter uses 120ms opacity/transform. New route navigation does not add motion; content changes in place.
- Loading feedback is inline and remains until the request settles; errors are shown beside the route browser.
- Honor `prefers-reduced-motion`; no new animation is required.

## 7. Depth & Surface

- Existing modal/form surfaces use a neutral fill, thin neutral outline and restrained shadow where the shared AppModal is used.
- SourcePicker's established surface colors and borders are the screenshot fidelity contract for this patch; do not restyle unrelated app surfaces.
- Nested controls use tighter radii than their containing modal.

## 8. Accessibility Constraints & Accepted Debt

### Constraints
- WCAG AA contrast for controls and text; all functionality keyboard reachable; every control has a visible focus state and an accessible name.
- Async loading/error/empty states are announced without moving focus.
- Folder search is labeled and supports Enter/Escape as expected; breadcrumb names identify each navigation level.

### Accepted Debt
| Item | Location | Why accepted | Exit |
|---|---|---|---|
| SourcePicker and newer AppModal use separate local style implementations | SourcePicker.svelte, AppModal.svelte | Existing migration is outside this Nuxeo route-picker task; preserve the screenshot's current picker styling | Consolidate only in a separately scoped UI task |
