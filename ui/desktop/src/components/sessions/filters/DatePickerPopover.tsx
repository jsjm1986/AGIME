import { memo, useState, useCallback, useMemo, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronLeft, ChevronRight, X } from 'lucide-react';
import { Button } from '../../ui/button';
import { cn } from '../../../utils';

interface DatePickerPopoverProps {
  selectedDates: Date[];
  onSelect: (dates: Date[]) => void;
  onClose: () => void;
  className?: string;
}

// Helper functions
const getDaysInMonth = (year: number, month: number): number => {
  return new Date(year, month + 1, 0).getDate();
};

const getFirstDayOfMonth = (year: number, month: number): number => {
  return new Date(year, month, 1).getDay();
};

const isSameDay = (date1: Date, date2: Date): boolean => {
  return date1.toDateString() === date2.toDateString();
};

const isDateInArray = (date: Date, dates: Date[]): boolean => {
  return dates.some((d) => isSameDay(d, date));
};

export const DatePickerPopover = memo(function DatePickerPopover({
  selectedDates: initialSelectedDates,
  onSelect,
  onClose,
  className,
}: DatePickerPopoverProps) {
  const { t, i18n } = useTranslation('sessions');
  const [currentMonth, setCurrentMonth] = useState(() => {
    const now = new Date();
    return { year: now.getFullYear(), month: now.getMonth() };
  });
  const [selectedDates, setSelectedDates] = useState<Date[]>(initialSelectedDates);
  const [rangeStart, setRangeStart] = useState<Date | null>(null);

  // Sync with props
  useEffect(() => {
    setSelectedDates(initialSelectedDates);
  }, [initialSelectedDates]);

  const monthNames = useMemo(() => {
    const locale = i18n.language === 'zh-CN' ? 'zh-CN' : 'en-US';
    return Array.from({ length: 12 }, (_, i) =>
      new Date(2024, i, 1).toLocaleDateString(locale, { month: 'long' })
    );
  }, [i18n.language]);

  const weekDays = useMemo(() => {
    const locale = i18n.language === 'zh-CN' ? 'zh-CN' : 'en-US';
    return Array.from({ length: 7 }, (_, i) =>
      new Date(2024, 0, i).toLocaleDateString(locale, { weekday: 'short' })
    );
  }, [i18n.language]);

  const calendarDays = useMemo(() => {
    const { year, month } = currentMonth;
    const daysInMonth = getDaysInMonth(year, month);
    const firstDay = getFirstDayOfMonth(year, month);
    const days: (Date | null)[] = [];

    // Add empty cells for days before first day
    for (let i = 0; i < firstDay; i++) {
      days.push(null);
    }

    // Add days of the month
    for (let day = 1; day <= daysInMonth; day++) {
      days.push(new Date(year, month, day));
    }

    return days;
  }, [currentMonth]);

  const handlePrevMonth = useCallback(() => {
    setCurrentMonth((prev) => {
      if (prev.month === 0) {
        return { year: prev.year - 1, month: 11 };
      }
      return { ...prev, month: prev.month - 1 };
    });
  }, []);

  const handleNextMonth = useCallback(() => {
    setCurrentMonth((prev) => {
      if (prev.month === 11) {
        return { year: prev.year + 1, month: 0 };
      }
      return { ...prev, month: prev.month + 1 };
    });
  }, []);

  const handleDateClick = useCallback(
    (date: Date, e: React.MouseEvent) => {
      e.preventDefault();

      if (e.shiftKey && rangeStart) {
        // Range selection with Shift
        const start = rangeStart < date ? rangeStart : date;
        const end = rangeStart < date ? date : rangeStart;
        const rangeDates: Date[] = [];

        const current = new Date(start);
        while (current <= end) {
          rangeDates.push(new Date(current));
          current.setDate(current.getDate() + 1);
        }

        // Add range dates to selection (without duplicates)
        setSelectedDates((prev) => {
          const newDates = [...prev];
          for (const d of rangeDates) {
            if (!isDateInArray(d, newDates)) {
              newDates.push(d);
            }
          }
          return newDates;
        });
        setRangeStart(null);
      } else if (e.ctrlKey || e.metaKey) {
        // Multi-select with Ctrl/Cmd
        setSelectedDates((prev) => {
          if (isDateInArray(date, prev)) {
            return prev.filter((d) => !isSameDay(d, date));
          }
          return [...prev, date];
        });
        setRangeStart(date);
      } else {
        // Single click - toggle selection or start new selection
        if (isDateInArray(date, selectedDates) && selectedDates.length === 1) {
          setSelectedDates([]);
        } else {
          setSelectedDates([date]);
        }
        setRangeStart(date);
      }
    },
    [rangeStart, selectedDates]
  );

  const handleClear = useCallback(() => {
    setSelectedDates([]);
    setRangeStart(null);
  }, []);

  const handleConfirm = useCallback(() => {
    onSelect(selectedDates);
    onClose();
  }, [selectedDates, onSelect, onClose]);

  const today = new Date();

  // Format selected dates for display
  const selectedSummary = useMemo(() => {
    if (selectedDates.length === 0) return '';
    if (selectedDates.length === 1) {
      return selectedDates[0].toLocaleDateString(i18n.language === 'zh-CN' ? 'zh-CN' : 'en-US', {
        month: 'short',
        day: 'numeric',
      });
    }
    const sorted = [...selectedDates].sort((a, b) => a.getTime() - b.getTime());
    const first = sorted[0].toLocaleDateString(i18n.language === 'zh-CN' ? 'zh-CN' : 'en-US', {
      month: 'short',
      day: 'numeric',
    });
    const last = sorted[sorted.length - 1].toLocaleDateString(
      i18n.language === 'zh-CN' ? 'zh-CN' : 'en-US',
      { month: 'short', day: 'numeric' }
    );
    return `${first} - ${last} (${selectedDates.length})`;
  }, [selectedDates, i18n.language]);

  return (
    <div
      className={cn(
        'fixed inset-0 z-[400] flex items-center justify-center bg-black/50',
        className
      )}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="bg-background-default border border-border-subtle rounded-lg shadow-lg p-4 min-w-[320px]">
        {/* Header */}
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-lg font-medium">{t('filters.datePicker.title')}</h3>
          <button
            onClick={onClose}
            className="p-1 hover:bg-background-muted rounded transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Month navigation */}
        <div className="flex items-center justify-between mb-4">
          <button
            onClick={handlePrevMonth}
            className="p-1 hover:bg-background-muted rounded transition-colors"
          >
            <ChevronLeft className="w-5 h-5" />
          </button>
          <span className="font-medium">
            {monthNames[currentMonth.month]} {currentMonth.year}
          </span>
          <button
            onClick={handleNextMonth}
            className="p-1 hover:bg-background-muted rounded transition-colors"
          >
            <ChevronRight className="w-5 h-5" />
          </button>
        </div>

        {/* Weekday headers */}
        <div className="grid grid-cols-7 gap-1 mb-2">
          {weekDays.map((day, i) => (
            <div key={i} className="text-center text-xs text-text-muted py-1">
              {day}
            </div>
          ))}
        </div>

        {/* Calendar grid */}
        <div className="grid grid-cols-7 gap-1">
          {calendarDays.map((date, i) => {
            if (!date) {
              return <div key={`empty-${i}`} className="h-8" />;
            }

            const isSelected = isDateInArray(date, selectedDates);
            const isToday = isSameDay(date, today);
            const isFuture = date > today;

            return (
              <button
                key={date.toISOString()}
                onClick={(e) => handleDateClick(date, e)}
                disabled={isFuture}
                className={cn(
                  'h-8 w-8 flex items-center justify-center text-sm rounded transition-colors',
                  isSelected && 'bg-blue-500 text-white hover:bg-blue-600',
                  !isSelected && !isFuture && 'hover:bg-background-muted',
                  isToday && !isSelected && 'ring-1 ring-blue-500',
                  isFuture && 'opacity-30 cursor-not-allowed'
                )}
              >
                {date.getDate()}
              </button>
            );
          })}
        </div>

        {/* Hints */}
        <div className="mt-3 text-xs text-text-muted space-y-1">
          <p>{t('filters.datePicker.rangeHint')}</p>
          <p>{t('filters.datePicker.multiHint')}</p>
        </div>

        {/* Selected summary */}
        {selectedDates.length > 0 && (
          <div className="mt-3 p-2 bg-background-muted rounded text-sm">
            {t('filters.datePicker.selected')}: {selectedSummary}
          </div>
        )}

        {/* Actions */}
        <div className="flex justify-between mt-4 pt-3 border-t border-border-subtle">
          <Button variant="ghost" size="sm" onClick={handleClear} disabled={selectedDates.length === 0}>
            {t('filters.datePicker.clear')}
          </Button>
          <Button variant="default" size="sm" onClick={handleConfirm}>
            {t('filters.datePicker.confirm')}
          </Button>
        </div>
      </div>
    </div>
  );
});

export default DatePickerPopover;
