import { invoke } from "@tauri-apps/api/core";
import "./TitleBar.css";

export default function TitleBar() {
  return (
    <div className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left" data-tauri-drag-region>
        <img src="/icon.png" className="titlebar-icon" alt="" />
        <span className="titlebar-title">rscapt</span>
      </div>
      <div className="titlebar-controls">
        <button className="wc wc-min" onClick={() => invoke("minimize_window")} title="Minimise">
          <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1" fill="currentColor"/></svg>
        </button>
        <button className="wc wc-max" onClick={() => invoke("maximize_window")} title="Maximise">
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
            <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor"/>
          </svg>
        </button>
        <button className="wc wc-close" onClick={() => invoke("close_window")} title="Close">
          <svg width="10" height="10" viewBox="0 0 10 10">
            <line x1="0" y1="0" x2="10" y2="10" stroke="currentColor" strokeWidth="1.2"/>
            <line x1="10" y1="0" x2="0" y2="10" stroke="currentColor" strokeWidth="1.2"/>
          </svg>
        </button>
      </div>
    </div>
  );
}
