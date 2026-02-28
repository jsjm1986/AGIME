import { fetchApi } from './client';

export interface BrandConfig {
  name: string;
  logoText: string;
  logoUrl: string | null;
  websiteUrl: string | null;
  websiteLabel: string | null;
  poweredByVisible: boolean;
  licensed: boolean;
  licensee: string | null;
  machineId: string;
}

const DEFAULT_BRAND: BrandConfig = {
  name: 'Agime Team',
  logoText: 'A',
  logoUrl: null,
  websiteUrl: 'https://www.agiatme.com',
  websiteLabel: 'Agime Official Website',
  poweredByVisible: true,
  licensed: false,
  licensee: null,
  machineId: '',
};

export async function fetchBrandConfig(): Promise<BrandConfig> {
  try {
    return await fetchApi<BrandConfig>('/api/brand/config');
  } catch {
    return DEFAULT_BRAND;
  }
}

export async function activateLicense(licenseKey: string): Promise<BrandConfig> {
  return fetchApi<BrandConfig>('/api/brand/activate', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ license_key: licenseKey }),
  });
}

export interface BrandOverrides {
  name?: string | null;
  logoText?: string | null;
  logoUrl?: string | null;
  websiteUrl?: string | null;
  websiteLabel?: string | null;
}

export async function fetchBrandOverrides(): Promise<BrandOverrides> {
  return fetchApi<BrandOverrides>('/api/brand/overrides');
}

export async function updateBrandOverrides(overrides: BrandOverrides): Promise<BrandConfig> {
  return fetchApi<BrandConfig>('/api/brand/overrides', {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(overrides),
  });
}

export { DEFAULT_BRAND };
