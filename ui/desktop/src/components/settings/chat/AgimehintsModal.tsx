import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { Check } from '../../icons';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';
import { isElectron } from '../../../platform';

// Support both .agimehints (preferred) and .goosehints (legacy) file names
const AGIME_HINTS_FILENAME = '.agimehints';
const GOOSE_HINTS_FILENAME = '.goosehints';

const HelpText = ({ t }: { t: (key: string) => string }) => (
  <div className="text-sm flex-col space-y-4 text-textSubtle">
    <p>
      {t('agimehints.helpText1')}
    </p>
    <p>
      {t('agimehints.helpText2')}
    </p>
    <p>
      {t('agimehints.helpText3')}{' '}
      <Button
        variant="link"
        className="text-blue-500 hover:text-blue-600 p-0 h-auto"
        onClick={() =>
          window.open('https://github.com/jsjm1986/AGIME', '_blank')
        }
      >
        {t('agimehints.docsLink')}
      </Button>{' '}
      {t('agimehints.helpText4')}
    </p>
  </div>
);

const ErrorDisplay = ({ error, t }: { error: Error; t: (key: string) => string }) => (
  <div className="text-sm text-textSubtle">
    <div className="text-red-600">{t('agimehints.readError')}: {JSON.stringify(error)}</div>
  </div>
);

const FileInfo = ({ filePath, found, t }: { filePath: string; found: boolean; t: (key: string) => string }) => (
  <div className="text-sm font-medium mb-2">
    {found ? (
      <div className="text-green-600">
        <Check className="w-4 h-4 inline-block" /> {t('agimehints.fileFound')}: {filePath}
      </div>
    ) : (
      <div>{t('agimehints.creatingFile')}: {filePath}</div>
    )}
  </div>
);

const getHintsFile = async (filePath: string) => await window.electron.readFile(filePath);

interface AgimehintsModalProps {
  directory: string;
  setIsAgimehintsModalOpen: (isOpen: boolean) => void;
}

export type { AgimehintsModalProps };

export const AgimehintsModal = ({ directory, setIsAgimehintsModalOpen }: AgimehintsModalProps) => {
  // AgimehintsModal is only available in Electron since it interacts with the filesystem
  if (!isElectron) {
    return null;
  }

  return <AgimehintsModalContent directory={directory} setIsAgimehintsModalOpen={setIsAgimehintsModalOpen} />;
};

const AgimehintsModalContent = ({ directory, setIsAgimehintsModalOpen }: AgimehintsModalProps) => {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  // Default to .agimehints for new files
  const [hintsFilePath, setHintsFilePath] = useState<string>(`${directory}/${AGIME_HINTS_FILENAME}`);
  const [hintsFile, setHintsFile] = useState<string>('');
  const [hintsFileFound, setHintsFileFound] = useState<boolean>(false);
  const [hintsFileReadError, setHintsFileReadError] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  // Track mounted state to prevent race conditions
  const isMountedRef = useRef(true);
  const saveTimeoutRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    // Reset mounted state on mount
    isMountedRef.current = true;

    const fetchHintsFile = async () => {
      try {
        // Try .agimehints first (preferred)
        const agimeHintsPath = `${directory}/${AGIME_HINTS_FILENAME}`;
        const agimeResult = await getHintsFile(agimeHintsPath);

        // Check if still mounted before updating state
        if (!isMountedRef.current) return;

        if (agimeResult.found) {
          setHintsFilePath(agimeHintsPath);
          setHintsFile(agimeResult.file);
          setHintsFileFound(true);
          setHintsFileReadError('');
          return;
        }

        // Fallback to .goosehints (legacy)
        const gooseHintsPath = `${directory}/${GOOSE_HINTS_FILENAME}`;
        const gooseResult = await getHintsFile(gooseHintsPath);

        // Check if still mounted before updating state
        if (!isMountedRef.current) return;

        if (gooseResult.found) {
          setHintsFilePath(gooseHintsPath);
          setHintsFile(gooseResult.file);
          setHintsFileFound(true);
          setHintsFileReadError('');
          return;
        }

        // Neither found, will create .agimehints for new files
        setHintsFilePath(agimeHintsPath);
        setHintsFile('');
        setHintsFileFound(false);
        setHintsFileReadError('');
      } catch (error) {
        if (!isMountedRef.current) return;
        console.error('Error fetching hints file:', error);
        setHintsFileReadError(t('agimehints.accessError'));
      }
    };

    if (directory) fetchHintsFile();

    // Cleanup function to mark component as unmounted and clear any pending timeouts
    return () => {
      isMountedRef.current = false;
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
    };
  }, [directory, t]);

  const writeFile = async () => {
    setIsSaving(true);
    setSaveSuccess(false);
    try {
      await window.electron.writeFile(hintsFilePath, hintsFile);
      if (!isMountedRef.current) return;
      setSaveSuccess(true);
      setHintsFileFound(true);
      // Clear any existing timeout before setting a new one
      if (saveTimeoutRef.current) {
        clearTimeout(saveTimeoutRef.current);
      }
      saveTimeoutRef.current = setTimeout(() => {
        if (isMountedRef.current) {
          setSaveSuccess(false);
        }
      }, 3000);
    } catch (error) {
      if (!isMountedRef.current) return;
      console.error('Error writing hints file:', error);
      setHintsFileReadError(t('agimehints.saveError'));
    } finally {
      if (isMountedRef.current) {
        setIsSaving(false);
      }
    }
  };

  return (
    <Dialog open={true} onOpenChange={(open) => setIsAgimehintsModalOpen(open)}>
      <DialogContent className="w-[80vw] max-w-[80vw] sm:max-w-[80vw] max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{t('agimehints.title')}</DialogTitle>
          <DialogDescription>
            {t('agimehints.description')}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pt-2 pb-4">
          <HelpText t={t} />

          <div>
            {hintsFileReadError ? (
              <ErrorDisplay error={new Error(hintsFileReadError)} t={t} />
            ) : (
              <div className="space-y-2">
                <FileInfo filePath={hintsFilePath} found={hintsFileFound} t={t} />
                <textarea
                  value={hintsFile}
                  className="w-full h-80 border rounded-md p-2 text-sm resize-none bg-background-default text-textStandard border-borderStandard focus:outline-none focus:ring-2 focus:ring-blue-500"
                  onChange={(event) => setHintsFile(event.target.value)}
                  placeholder={t('agimehints.placeholder')}
                />
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          {saveSuccess && (
            <span className="text-green-600 text-sm flex items-center gap-1 mr-auto">
              <Check className="w-4 h-4" />
              {t('agimehints.savedSuccess')}
            </span>
          )}
          <Button variant="outline" onClick={() => setIsAgimehintsModalOpen(false)}>
            {tCommon('close')}
          </Button>
          <Button onClick={writeFile} disabled={isSaving}>
            {isSaving ? tCommon('saving') : tCommon('save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

// Backward compatibility
export { AgimehintsModal as GoosehintsModal };
export type { AgimehintsModalProps as GoosehintsModalProps };
