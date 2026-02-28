import { createContext, useContext, useEffect, useState, useCallback, ReactNode } from 'react';
import { BrandConfig, BrandOverrides, DEFAULT_BRAND, fetchBrandConfig, activateLicense, fetchBrandOverrides, updateBrandOverrides } from '../api/brand';

interface BrandContextValue {
  brand: BrandConfig;
  activate: (key: string) => Promise<BrandConfig>;
  overrides: BrandOverrides | null;
  saveOverrides: (o: BrandOverrides) => Promise<BrandConfig>;
}

const BrandContext = createContext<BrandContextValue>({
  brand: DEFAULT_BRAND,
  activate: () => Promise.reject('Not initialized'),
  overrides: null,
  saveOverrides: () => Promise.reject('Not initialized'),
});

export function BrandProvider({ children }: { children: ReactNode }) {
  const [brand, setBrand] = useState<BrandConfig>(DEFAULT_BRAND);
  const [overrides, setOverrides] = useState<BrandOverrides | null>(null);

  useEffect(() => {
    fetchBrandConfig().then((b) => {
      setBrand(b);
      if (b.licensed) {
        fetchBrandOverrides().then(setOverrides).catch(() => {});
      }
    });
  }, []);

  useEffect(() => {
    document.title = `${brand.name} Admin`;
  }, [brand.name]);

  const activate = useCallback(async (key: string) => {
    const newBrand = await activateLicense(key);
    setBrand(newBrand);
    if (newBrand.licensed) {
      fetchBrandOverrides().then(setOverrides).catch(() => {});
    }
    return newBrand;
  }, []);

  const saveOverrides = useCallback(async (o: BrandOverrides) => {
    const newBrand = await updateBrandOverrides(o);
    setBrand(newBrand);
    setOverrides(o);
    return newBrand;
  }, []);

  return (
    <BrandContext.Provider value={{ brand, activate, overrides, saveOverrides }}>
      {children}
    </BrandContext.Provider>
  );
}

export function useBrand(): BrandContextValue {
  return useContext(BrandContext);
}
