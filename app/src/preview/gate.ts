// The scripted preview backend is for `npm run dev` only, and only when the URL asks for a scenario.
export function previewRequested(dev: boolean, search: string) {
  return dev && new URLSearchParams(search).has('scenario');
}
