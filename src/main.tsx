import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { ErrorBoundary } from './ErrorBoundary'
import { I18nProvider } from './i18n'
import './index.css'
import App from './App.tsx'

const root = document.getElementById('root');
if (root) {
  createRoot(root).render(
    <StrictMode>
      <ErrorBoundary>
        <I18nProvider>
          <App />
        </I18nProvider>
      </ErrorBoundary>
    </StrictMode>,
  );
} else {
  document.body.innerHTML = '<div style="color:#e0e0e0;padding:2rem">DIT Pro: #root element not found</div>';
}
