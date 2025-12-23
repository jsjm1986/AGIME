import { getConfigCompat } from './utils/envCompat';

export const getApiUrl = (endpoint: string): string => {
  const gooseApiHost = String(getConfigCompat('API_HOST') || '');
  const cleanEndpoint = endpoint.startsWith('/') ? endpoint : `/${endpoint}`;
  return `${gooseApiHost}${cleanEndpoint}`;
};
