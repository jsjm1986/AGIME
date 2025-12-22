/**
 * Web-specific React entry point
 *
 * This entry point handles:
 * 1. Authentication via URL hash parameter (not query param - prevents server logging)
 * 2. API client configuration
 * 3. App initialization for web browser
 */

// IMPORTANT: Import platform first to install window.electron shim before other modules load
import { platform, setTunnelSecret, isAuthenticated } from './platform';

import React, { Suspense, lazy, useEffect, useState } from 'react';
import ReactDOM from 'react-dom/client';
import { ConfigProvider } from './components/ConfigContext';
import { ThinkingVisibilityProvider } from './contexts/ThinkingVisibilityContext';
import { ErrorBoundary } from './components/ErrorBoundary';
import SuspenseLoader from './suspense-loader';
import { client } from './api/client.gen';
import AuthRequired from './components/AuthRequired';

// Initialize i18n
import './i18n';

// Import web-specific styles
import './styles/web-overrides.css';

// Add web-platform class to html element for CSS targeting
document.documentElement.classList.add('web-platform');
document.body.classList.add('web-platform');

const App = lazy(() => import('./App'));

/**
 * Extract and process authentication secret from URL
 * Supports both URL hash (#secret=xxx) and query param (?secret=xxx)
 * Hash is preferred as it's not sent to the server
 */
function processAuthentication(): boolean {
  // First check URL hash (preferred - not sent to server)
  const hash = window.location.hash;
  if (hash) {
    const hashParams = new URLSearchParams(hash.slice(1));
    const hashSecret = hashParams.get('secret');
    if (hashSecret) {
      setTunnelSecret(hashSecret);
      // Clear the hash
      window.history.replaceState({}, '', window.location.pathname + window.location.search);
      console.log('[Web] Authentication secret stored from hash');
      return true;
    }
  }

  // Fallback to query parameter (for backwards compatibility)
  const urlParams = new URLSearchParams(window.location.search);
  const querySecret = urlParams.get('secret');

  if (querySecret) {
    // Store the secret
    setTunnelSecret(querySecret);

    // Remove secret from URL for security (don't expose in browser history)
    urlParams.delete('secret');
    const newSearch = urlParams.toString();
    const newUrl = window.location.pathname + (newSearch ? `?${newSearch}` : '') + window.location.hash;
    window.history.replaceState({}, '', newUrl);

    console.log('[Web] Authentication secret stored from query param');
    return true;
  }

  return isAuthenticated();
}

/**
 * Configure API client for web
 */
async function configureApiClient(): Promise<void> {
  // On web, API is served from the same origin
  const baseUrl = window.location.origin;

  // Get secret from platform (uses in-memory storage)
  const secret = await platform.getSecretKey();

  console.log('[Web] Configuring API client for', baseUrl);

  client.setConfig({
    baseUrl,
    headers: {
      'Content-Type': 'application/json',
      'X-Secret-Key': secret,
    },
  });
}

/**
 * Web App Wrapper - handles authentication state
 */
function WebAppWrapper() {
  const [authState, setAuthState] = useState<'checking' | 'authenticated' | 'unauthenticated'>('checking');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    async function init() {
      try {
        // Check for authentication
        const isAuth = processAuthentication();

        if (!isAuth) {
          setAuthState('unauthenticated');
          return;
        }

        // Configure API client
        await configureApiClient();

        setAuthState('authenticated');
      } catch (err) {
        console.error('[Web] Initialization error:', err);
        setError(err instanceof Error ? err.message : 'Unknown error');
        setAuthState('unauthenticated');
      }
    }

    init();
  }, []);

  // Show loading state while checking auth
  if (authState === 'checking') {
    return SuspenseLoader();
  }

  // Show auth required screen if not authenticated
  if (authState === 'unauthenticated') {
    return <AuthRequired error={error} />;
  }

  // Render the main app
  return (
    <Suspense fallback={SuspenseLoader()}>
      <ConfigProvider>
        <ThinkingVisibilityProvider>
          <ErrorBoundary>
            <App />
          </ErrorBoundary>
        </ThinkingVisibilityProvider>
      </ConfigProvider>
    </Suspense>
  );
}

// Render the app
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <WebAppWrapper />
  </React.StrictMode>
);
