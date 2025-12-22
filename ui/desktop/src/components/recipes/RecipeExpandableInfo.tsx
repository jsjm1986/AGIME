import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown } from 'lucide-react';
import { Button } from '../ui/button';

interface RecipeExpandableInfoProps {
  infoLabel: string;
  infoValue: string;
  required?: boolean;
  onClickEdit: () => void;
}

export default function RecipeExpandableInfo({
  infoValue,
  infoLabel,
  required = false,
  onClickEdit,
}: RecipeExpandableInfoProps) {
  const { t } = useTranslation('recipes');
  const [isValueExpanded, setValueExpanded] = useState(false);
  const [isClamped, setIsClamped] = useState(false);
   
  const contentRef = useRef<HTMLParagraphElement>(null);
  const measureRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = measureRef.current;
    if (el) {
      const lineHeight = parseFloat(window.getComputedStyle(el).lineHeight || '0');
      const maxHeight = lineHeight * 3;
      const actualHeight = el.scrollHeight;
      setIsClamped(actualHeight > maxHeight);
    }
  }, [infoValue]);

  return (
    <>
      <div className="flex justify-between items-center mb-2">
        <label className="block text-md text-textProminent font-bold">
          {infoLabel} {required && <span className="text-red-500">*</span>}
        </label>
      </div>

      <div className="relative rounded-lg bg-background-default text-textStandard">
        {infoValue && (
          <>
            <div
              ref={measureRef}
              className="invisible absolute whitespace-pre-wrap w-full pointer-events-none"
              style={{ position: 'absolute', top: '-9999px' }}
            >
              {infoValue}
            </div>

            <p
              ref={contentRef}
              className={`whitespace-pre-wrap transition-all duration-300 ${
                !isValueExpanded ? 'line-clamp-3' : ''
              }`}
            >
              {infoValue}
            </p>
          </>
        )}
        <div className="mt-4 flex items-center justify-between">
          <Button
            type="button"
            onClick={(e) => {
              e.preventDefault();
              setValueExpanded(true);
              onClickEdit();
            }}
            className="w-36 px-3 py-3 bg-background-defaultInverse text-sm text-textProminentInverse rounded-xl hover:bg-bgStandardInverse transition-colors"
          >
            {infoValue ? t('expandableInfo.edit') : t('expandableInfo.add')} {infoLabel.toLowerCase()}
          </Button>

          {infoValue && isClamped && (
            <Button
              type="button"
              variant="ghost"
              shape="round"
              onClick={() => setValueExpanded(!isValueExpanded)}
              aria-label={isValueExpanded ? t('expandableInfo.collapseContent') : t('expandableInfo.expandContent')}
              title={isValueExpanded ? t('expandableInfo.collapse') : t('expandableInfo.expand')}
              className="bg-background-muted hover:bg-background-default text-text-muted hover:text-text-default transition-colors"
            >
              <ChevronDown
                className={`w-6 h-6 transition-transform duration-300 ${
                  isValueExpanded ? 'rotate-180' : ''
                }`}
                strokeWidth={2.5}
              />
            </Button>
          )}
        </div>
      </div>
    </>
  );
}
