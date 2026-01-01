import { memo, useCallback } from 'react';
import { Globe, MapPin, ArrowRightLeft } from 'lucide-react';
import { CollapsibleSection } from '../components/CollapsibleSection';
import { ProviderCardGrid } from '../components/ProviderCard';
import {
  internationalProviders,
  chinaProviders,
  proxyProviders,
  type ProviderPreset,
} from '../data/providerPresets';

interface ProviderSelectProps {
  selectedProvider: ProviderPreset | null;
  onSelect: (provider: ProviderPreset) => void;
}

export const ProviderSelect = memo(function ProviderSelect({
  selectedProvider,
  onSelect,
}: ProviderSelectProps) {
  const handleSelect = useCallback((provider: ProviderPreset) => {
    onSelect(provider);
  }, [onSelect]);

  return (
    <div className="space-y-2">
      {/* International Providers */}
      <CollapsibleSection
        title="国际服务"
        icon={<Globe className="w-4 h-4" />}
        defaultExpanded={true}
        badge={internationalProviders.length}
      >
        <ProviderCardGrid
          providers={internationalProviders}
          selectedId={selectedProvider?.id}
          onSelect={handleSelect}
        />
      </CollapsibleSection>

      {/* China Providers */}
      <CollapsibleSection
        title="国内服务"
        icon={<MapPin className="w-4 h-4" />}
        defaultExpanded={true}
        badge={chinaProviders.length}
      >
        <ProviderCardGrid
          providers={chinaProviders}
          selectedId={selectedProvider?.id}
          onSelect={handleSelect}
        />
      </CollapsibleSection>

      {/* Proxy Providers */}
      <CollapsibleSection
        title="第三方/代理"
        icon={<ArrowRightLeft className="w-4 h-4" />}
        defaultExpanded={false}
        badge={proxyProviders.length}
      >
        <ProviderCardGrid
          providers={proxyProviders}
          selectedId={selectedProvider?.id}
          onSelect={handleSelect}
        />
      </CollapsibleSection>
    </div>
  );
});

export default ProviderSelect;
