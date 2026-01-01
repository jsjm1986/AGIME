import { memo } from 'react';
import { Check } from 'lucide-react';
import { cn } from '../../../../utils';
import type { ProviderPreset } from '../data/providerPresets';

interface QuickSetupProviderCardProps {
  provider: ProviderPreset;
  isSelected?: boolean;
  onClick: (provider: ProviderPreset) => void;
}

export const QuickSetupProviderCard = memo(function QuickSetupProviderCard({
  provider,
  isSelected = false,
  onClick,
}: QuickSetupProviderCardProps) {
  return (
    <button
      type="button"
      onClick={() => onClick(provider)}
      className={cn(
        'group relative flex flex-col items-start justify-between p-3 rounded-xl transition-all duration-300',
        'min-w-[100px] min-h-[76px]',
        'focus:outline-none focus-visible:ring-2 focus-visible:ring-block-teal focus-visible:ring-offset-2',
        isSelected
          ? 'bg-gradient-to-br from-block-teal/20 to-block-orange/10 border-2 border-block-teal shadow-md'
          : 'bg-background-default border border-border-default hover:border-block-teal/50 hover:shadow-lg hover:shadow-block-teal/10'
      )}
      data-testid={`quick-setup-provider-${provider.id}`}
    >
      {/* Selection indicator */}
      {isSelected && (
        <div className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-block-teal flex items-center justify-center shadow-md">
          <Check className="w-3 h-3 text-white" strokeWidth={3} />
        </div>
      )}

      <div className="flex-1 w-full">
        <span className={cn(
          'text-sm font-semibold leading-tight block',
          isSelected ? 'text-block-teal' : 'text-text-default group-hover:text-block-teal'
        )}>
          {provider.displayName}
        </span>
        <span className="text-xs text-text-muted mt-1.5 line-clamp-2 text-left block leading-relaxed">
          {provider.description}
        </span>
      </div>
    </button>
  );
});

// Grid container for provider cards
interface ProviderCardGridProps {
  providers: ProviderPreset[];
  selectedId?: string;
  onSelect: (provider: ProviderPreset) => void;
}

export const ProviderCardGrid = memo(function ProviderCardGrid({
  providers,
  selectedId,
  onSelect,
}: ProviderCardGridProps) {
  return (
    <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-2">
      {providers.map(provider => (
        <QuickSetupProviderCard
          key={provider.id}
          provider={provider}
          isSelected={selectedId === provider.id}
          onClick={onSelect}
        />
      ))}
    </div>
  );
});

export default QuickSetupProviderCard;
