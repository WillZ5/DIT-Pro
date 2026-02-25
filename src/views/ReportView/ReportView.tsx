export function ReportView() {
  return (
    <div className="report-view">
      <div className="view-header">
        <h2>Reports</h2>
      </div>

      <div className="empty-state">
        <div className="empty-icon">📊</div>
        <h3>No reports yet</h3>
        <p>Reports will be generated after completing offload jobs.</p>
      </div>
    </div>
  );
}
