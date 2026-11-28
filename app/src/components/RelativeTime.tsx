const formatter = new Intl.RelativeTimeFormat('en', { numeric: 'always' });

export function relativeTime(value: number, now = Date.now()): string {
  if (!Number.isFinite(value) || !Number.isFinite(new Date(value).getTime())) return 'Time unknown';
  const elapsed = now - value;
  if (Math.abs(elapsed) < 60_000) return 'Just now';
  const [unit, size] = Math.abs(elapsed) < 3_600_000 ? ['minute', 60_000] as const
    : Math.abs(elapsed) < 86_400_000 ? ['hour', 3_600_000] as const : ['day', 86_400_000] as const;
  return formatter.format(-Math.trunc(elapsed / size), unit);
}

// Ages refresh with the existing view; the exact timestamp is always available on hover.
// inline: the age follows other words in a sentence ("Checked just now"), so it starts in lower case.
export function RelativeTime({ value, inline = false }: { value: number; inline?: boolean }) {
  const date = new Date(value);
  const exact = date.toLocaleString(), relative = relativeTime(value);
  const age = inline ? relative.charAt(0).toLowerCase() + relative.slice(1) : relative;
  return Number.isFinite(date.getTime())
    ? <time className="coming-control" tabIndex={0} dateTime={date.toISOString()} aria-label={`${age}. ${exact}`}>
      {age}<span className="control-tip" aria-hidden="true">{exact}</span>
    </time>
    : <>Time unknown</>;
}
