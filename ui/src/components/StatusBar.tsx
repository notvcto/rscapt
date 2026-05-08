import "./StatusBar.css";

export default function StatusBar({ connected }: { connected: boolean }) {
  return (
    <div className="statusbar">
      <span className={`status-dot ${connected ? "on" : "off"}`} />
      <span className="status-text">
        {connected ? "daemon: connected" : "daemon: connecting…"}
      </span>
    </div>
  );
}
