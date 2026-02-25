import { useState } from "react";
import type { Job } from "../../types";

export function JobsView() {
  const [jobs] = useState<Job[]>([]);

  return (
    <div className="jobs-view">
      <div className="view-header">
        <h2>Jobs</h2>
        <button className="btn-primary" onClick={() => {/* TODO: new offload job */}}>
          + New Offload
        </button>
      </div>

      {jobs.length === 0 ? (
        <div className="empty-state">
          <div className="empty-icon">📁</div>
          <h3>No active jobs</h3>
          <p>Insert a card and click "New Offload" to start copying.</p>
        </div>
      ) : (
        <div className="jobs-list">
          {jobs.map((job) => (
            <div key={job.id} className="job-card">
              <div className="job-info">
                <span className="job-name">{job.name}</span>
                <span className="job-status">{job.status}</span>
              </div>
              <div className="job-progress">
                <div
                  className="progress-bar"
                  style={{
                    width: `${job.totalBytes > 0 ? (job.copiedBytes / job.totalBytes) * 100 : 0}%`,
                  }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
