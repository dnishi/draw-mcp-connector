import { CanvasPage } from '@/features/canvas/CanvasPage';
import {
  hashParams,
  parseConnectionInfo,
} from '@/features/sync/connectionInfo';

// Connection info is after # in the URL (the QR code URL shown by the bridge on the PC).
// It does not change after being read, so read it once at startup
const connectionInfo = parseConnectionInfo(hashParams(window.location.hash));

function App() {
  return <CanvasPage connectionInfo={connectionInfo} />;
}

export default App;
