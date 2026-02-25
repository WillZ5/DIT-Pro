export function SettingsView() {
  return (
    <div className="settings-view">
      <div className="view-header">
        <h2>Settings</h2>
      </div>

      <div className="settings-sections">
        <section className="settings-section">
          <h3>Hash Algorithms</h3>
          <p>Configure which hash algorithms to use during copy verification.</p>
        </section>

        <section className="settings-section">
          <h3>Email Notifications</h3>
          <p>Configure SMTP settings for copy completion alerts.</p>
        </section>

        <section className="settings-section">
          <h3>IO Scheduling</h3>
          <p>Per-device concurrency settings for optimal performance.</p>
        </section>
      </div>
    </div>
  );
}
