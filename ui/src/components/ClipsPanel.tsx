import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Clip } from "../App";
import "./ClipsPanel.css";

function sizeLabel(bytes: number): string {
  const mb = bytes / 1_048_576;
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${mb.toFixed(0)} MB`;
}

export default function ClipsPanel({ clips }: { clips: Clip[] }) {
  const [selected, setSelected] = useState<string | null>(null);

  const share = (path: string) =>
    invoke("send_cmd", { msg: { type: "Share", clip_path: path, expiry: "1w" } });

  return (
    <div className="panel clips-panel">
      <div className="panel-header">
        <span className="panel-title">Clips</span>
        <span className="panel-count">{clips.length}</span>
      </div>
      <div className="panel-body">
        {clips.length === 0 && (
          <p className="empty">No clips yet — processed clips will appear here.</p>
        )}
        {clips.map((clip) => (
          <div
            key={clip.path}
            className={`clip-row ${selected === clip.path ? "selected" : ""}`}
            onClick={() => setSelected(selected === clip.path ? null : clip.path)}
          >
            <div className="clip-main">
              <span className="clip-name">{clip.filename}</span>
              <span className="clip-size">{sizeLabel(clip.size_bytes)}</span>
            </div>
            {clip.share_url && (
              <a className="clip-share-pill" href={clip.share_url} target="_blank" rel="noreferrer">
                {clip.share_url}
              </a>
            )}
            {selected === clip.path && (
              <div className="clip-actions">
                <button className="clip-btn" onClick={(e) => { e.stopPropagation(); }}>Post-process</button>
                <button className="clip-btn" onClick={(e) => { e.stopPropagation(); }}>Compress</button>
                <button className="clip-btn accent" onClick={(e) => { e.stopPropagation(); share(clip.path); }}>Share</button>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
