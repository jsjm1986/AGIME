import { useState, useEffect } from 'react';
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

const HelpText = ({ t }: { t: (key: string) => string }) => (
  <div className="text-sm flex-col space-y-4 text-textSubtle">
    <p>
      {t('goosehints.helpText1')}
    </p>
    <p>
      {t('goosehints.helpText2')}
    </p>
    <p>
      {t('goosehints.helpText3')}{' '}
      <Button
        variant="link"
        className="text-blue-500 hover:text-blue-600 p-0 h-auto"
        onClick={() =>
          window.open('https://block.github.io/goose/docs/guides/using-goosehints/', '_blank')
        }
      >
        {t('goosehints.docsLink')}
      </Button>{' '}
      {t('goosehints.helpText4')}
    </p>
  </div>
);

const ErrorDisplay = ({ error, t }: { error: Error; t: (key: string) => string }) => (
  <div className="text-sm text-textSubtle">
    <div className="text-red-600">{t('goosehints.readError')}: {JSON.stringify(error)}</div>
  </div>
);

const FileInfo = ({ filePath, found, t }: { filePath: string; found: boolean; t: (key: string) => string }) => (
  <div className="text-sm font-medium mb-2">
    {found ? (
      <div className="text-green-600">
        <Check className="w-4 h-4 inline-block" /> {t('goosehints.fileFound')}: {filePath}
      </div>
    ) : (
      <div>{t('goosehints.creatingFile')}: {filePath}</div>
    )}
  </div>
);

const getGoosehintsFile = async (filePath: string) => await window.electron.readFile(filePath);

interface GoosehintsModalProps {
  directory: string;
  setIsGoosehintsModalOpen: (isOpen: boolean) => void;
}

export const GoosehintsModal = ({ directory, setIsGoosehintsModalOpen }: GoosehintsModalProps) => {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const goosehintsFilePath = `${directory}/.goosehints`;
  const [goosehintsFile, setGoosehintsFile] = useState<string>('');
  const [goosehintsFileFound, setGoosehintsFileFound] = useState<boolean>(false);
  const [goosehintsFileReadError, setGoosehintsFileReadError] = useState<string>('');
  const [isSaving, setIsSaving] = useState(false);
  const [saveSuccess, setSaveSuccess] = useState(false);

  useEffect(() => {
    const fetchGoosehintsFile = async () => {
      try {
        const { file, error, found } = await getGoosehintsFile(goosehintsFilePath);
        setGoosehintsFile(file);
        setGoosehintsFileFound(found);
        setGoosehintsFileReadError(found && error ? error : '');
      } catch (error) {
        console.error('Error fetching .goosehints file:', error);
        setGoosehintsFileReadError(t('goosehints.accessError'));
      }
    };
    if (directory) fetchGoosehintsFile();
  }, [directory, goosehintsFilePath, t]);

  const writeFile = async () => {
    setIsSaving(true);
    setSaveSuccess(false);
    try {
      await window.electron.writeFile(goosehintsFilePath, goosehintsFile);
      setSaveSuccess(true);
      setGoosehintsFileFound(true);
      setTimeout(() => setSaveSuccess(false), 3000);
    } catch (error) {
      console.error('Error writing .goosehints file:', error);
      setGoosehintsFileReadError(t('goosehints.saveError'));
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <Dialog open={true} onOpenChange={(open) => setIsGoosehintsModalOpen(open)}>
      <DialogContent className="w-[80vw] max-w-[80vw] sm:max-w-[80vw] max-h-[90vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{t('goosehints.title')}</DialogTitle>
          <DialogDescription>
            {t('goosehints.description')}
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 overflow-y-auto space-y-4 pt-2 pb-4">
          <HelpText t={t} />

          <div>
            {goosehintsFileReadError ? (
              <ErrorDisplay error={new Error(goosehintsFileReadError)} t={t} />
            ) : (
              <div className="space-y-2">
                <FileInfo filePath={goosehintsFilePath} found={goosehintsFileFound} t={t} />
                <textarea
                  value={goosehintsFile}
                  className="w-full h-80 border rounded-md p-2 text-sm resize-none bg-background-default text-textStandard border-borderStandard focus:outline-none focus:ring-2 focus:ring-blue-500"
                  onChange={(event) => setGoosehintsFile(event.target.value)}
                  placeholder={t('goosehints.placeholder')}
                />
              </div>
            )}
          </div>
        </div>

        <DialogFooter>
          {saveSuccess && (
            <span className="text-green-600 text-sm flex items-center gap-1 mr-auto">
              <Check className="w-4 h-4" />
              {t('goosehints.savedSuccess')}
            </span>
          )}
          <Button variant="outline" onClick={() => setIsGoosehintsModalOpen(false)}>
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
