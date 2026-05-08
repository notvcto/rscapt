import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import TitleBar from "./components/TitleBar";
import JobsPanel from "./components/JobsPanel";
import ClipsPanel from "./components/ClipsPanel";
import StatusBar from "./components/StatusBar";
import "./App.css";

export type JobStatus =
  | "Queued" | "Running" | "Done" | "Failed" | "Cancelled";

export interface Job {
  id: string;
  kind: string;
  source: string;
  output: string;
  status: JobStatus | { Failed: string };
  progress: number;
  share_url?: string;
}

export interface Clip {
  path: string;
  filename: string;
  size_bytes: number;
  share_url?: string;
}

export default function App() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [clips, setClips] = useState<Clip[]>([]);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    const unlisten: (() => void)[] = [];

    listen<Record<string, unknown>>("daemon-message", ({ payload }) => {
      const msg = payload as Record<string, unknown>;
      switch (msg.type) {
        case "Snapshot":
          setJobs((msg.jobs as Job[]) ?? []);
          break;
        case "JobUpdate":
          setJobs((prev) => {
            const job = msg.job as Job;
            const idx = prev.findIndex((j) => j.id === job.id);
            if (idx === -1) return [job, ...prev];
            const next = [...prev];
            next[idx] = job;
            return next;
          });
          break;
        case "ClipLibrary":
        case "ClipUpdated":
          setClips((msg.clips as Clip[]) ?? []);
          break;
      }
    }).then((u) => unlisten.push(u));

    listen("daemon-connected", () => setConnected(true))
      .then((u) => unlisten.push(u));
    listen("daemon-disconnected", () => setConnected(false))
      .then((u) => unlisten.push(u));

    return () => unlisten.forEach((u) => u());
  }, []);

  return (
    <div className="app">
      <TitleBar />
      <div className="panels">
        <JobsPanel jobs={jobs} />
        <div className="divider" />
        <ClipsPanel clips={clips} />
      </div>
      <StatusBar connected={connected} />
    </div>
  );
}
