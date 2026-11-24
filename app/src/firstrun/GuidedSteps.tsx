import { guidedSteps } from './guided';

export default function GuidedSteps({ current }: { current: number }) {
  return <ol className="guided-steps" aria-label="Setup progress">
    {guidedSteps.map((name, index) => <li key={name} className={index < current ? 'done' : index === current ? 'current' : undefined} aria-current={index === current ? 'step' : undefined}>
      <span className="guided-step-dot" aria-hidden="true">{index < current
        ? <svg viewBox="0 0 16 16"><path d="m3.5 8.5 3 3 6-7" /></svg>
        : index + 1}</span>
      {name}{index < current && <span className="sr-only">, done</span>}
    </li>)}
  </ol>;
}
