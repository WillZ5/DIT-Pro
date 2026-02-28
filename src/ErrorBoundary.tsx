import { Component } from "react";
import type { ReactNode, ErrorInfo } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[DIT Pro] React error:", error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            height: "100vh",
            background: "#09090b",
            color: "#e0e0e0",
            fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif",
            padding: "2rem",
            textAlign: "center",
          }}
        >
          <div style={{ fontSize: "48px", marginBottom: "1rem" }}>!</div>
          <h1 style={{ fontSize: "1.25rem", marginBottom: "0.5rem" }}>
            DIT Pro — Render Error
          </h1>
          <p style={{ color: "#a1a1aa", marginBottom: "1rem", maxWidth: "480px" }}>
            {this.state.error?.message || "An unexpected error occurred."}
          </p>
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: "8px 20px",
              background: "#3b82f6",
              color: "#fff",
              border: "none",
              borderRadius: "6px",
              cursor: "pointer",
              fontSize: "0.875rem",
            }}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
