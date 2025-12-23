import React, { Suspense, lazy } from 'react';
import ReactDOM from 'react-dom/client';
import { ConfigProvider } from './components/ConfigContext';
import { ThinkingVisibilityProvider } from './contexts/ThinkingVisibilityContext';
import { ErrorBoundary } from './components/ErrorBoundary';
import SuspenseLoader from './suspense-loader';
import { client } from './api/client.gen';

// Initialize i18n
import './i18n';

const App = lazy(() => import('./App'));

(async () => {
  // Check if we're in the launcher view (doesn't need agimed connection)
  const isLauncher = window.location.hash === '#/launcher';

  if (!isLauncher) {
    console.log('window created, getting agimed connection info');
    const agimeApiHost = await window.electron.getAgimedHostPort();
    if (agimeApiHost === null) {
      window.alert('failed to start AGIME backend process');
      return;
    }
    console.log('connecting at', agimeApiHost);
    client.setConfig({
      baseUrl: agimeApiHost,
      headers: {
        'Content-Type': 'application/json',
        'X-Secret-Key': await window.electron.getSecretKey(),
      },
    });
  }

  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <Suspense fallback={SuspenseLoader()}>
        <ConfigProvider>
          <ThinkingVisibilityProvider>
            <ErrorBoundary>
              <App />
            </ErrorBoundary>
          </ThinkingVisibilityProvider>
        </ConfigProvider>
      </Suspense>
    </React.StrictMode>
  );
})();
