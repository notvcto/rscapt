import { invoke } from "@tauri-apps/api/core";
import type { Job } from "../App";
import "./JobsPanel.css";

const KIND_LABELS: Record<string, string> = {
  Upscale: "UPSCALE",
  PostProcess: "POST",
  Compress: "COMPRESS",
  Share: "SHARE",
};

function kindLabel(job: Job): string {
  if (typeof job.kind === "object") {
    const key = Object.keys(job.kind)[0];
    return KIND_LABELS[key] ?? key;
  }
  return KIND_LABELS[job.kind as string] ?? String(job.kind);
}

function statusText(job: Job): string {
  if (typeof job.status === "object" && "Failed" in job.status) {
    return `Failed: ${job.status.Failed}`;
  }
  return job.status as string;
}

function isRunning(job: Job): boolean {
  return job.status === "Running";
}

function isFailed(job: Job): boolean {
  return typeof job.status === "object" && "Failed" in job.status;
}

export default function JobsPanel({ jobs }: { jobs: Job[] }) {
  const cancel = (id: string) =>
    invoke("send_cmd", { msg: { type: "Cancel", id } });

  return (
    <div className="panel jobs-panel">
      <div className="panel-header">
        <span className="panel-title">Jobs</span>
        <span className="panel-count">{jobs.length}</span>
      </div>
      <div className="panel-body">
        {jobs.length === 0 && (
          <p className="empty">No jobs yet — save a replay buffer clip in OBS.</p>
        )}
        {jobs.map((job) => (
          <div
            key={job.id}
            className={`job-card ${isFailed(job) ? "failed" : ""} ${job.status === "Done" ? "done" : ""}`}
          >
            <div className="job-top">
              <span className="job-badge">{kindLabel(job)}</span>
              <span className="job-name">
                {job.source.split(/[\\/]/).pop()}
              </span>
              {isRunning(job) && (
                <button className="job-cancel" onClick={() => cancel(job.id)} title="Cancel">✕</button>
              )}
            </div>
            <div className="job-progress-track">
              <div
                className="job-progress-fill"
                style={{ width: `${job.progress}%` }}
              />
            </div>
            <div className="job-status">{statusText(job)}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
