import { Button } from '../components/Button';

// The guided first run's way out, offered only where setup has stopped: it ends guidance and shows all of Bridge.
export default function SetUpLater({ onClick }: { onClick: () => void }) {
  return <Button type="button" kind="quiet" className="guided-later" onClick={onClick}>Set up later</Button>;
}
