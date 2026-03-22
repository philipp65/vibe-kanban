import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@vibe/ui/components/KeyboardDialog';
import { Button } from '@vibe/ui/components/Button';
import { create, useModal } from '@ebay/nice-modal-react';
import { defineModal } from '@/shared/lib/modals';
import { useTranslation } from 'react-i18next';

const GlabCliSetupDialogImpl = create<Record<string, never>>(() => {
  const modal = useModal();
  const { t } = useTranslation();

  const handleClose = () => {
    modal.resolve(undefined);
    modal.hide();
  };

  return (
    <Dialog
      open={modal.visible}
      onOpenChange={(open) => !open && handleClose()}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {t('settings:integrations.gitlab.cliSetup.title')}
          </DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <p>{t('settings:integrations.gitlab.cliSetup.description')}</p>

          <div className="space-y-3">
            <p className="text-sm font-medium">
              {t('settings:integrations.gitlab.cliSetup.installTitle')}
            </p>

            <div className="space-y-2 ml-2">
              <div>
                <p className="text-sm text-muted-foreground">
                  {t('settings:integrations.gitlab.cliSetup.install.macOs')}
                </p>
                <pre className="rounded bg-muted px-2 py-1 text-xs">
                  brew install glab
                </pre>
              </div>

              <div>
                <p className="text-sm text-muted-foreground">
                  {t('settings:integrations.gitlab.cliSetup.install.windows')}
                </p>
                <pre className="rounded bg-muted px-2 py-1 text-xs">
                  winget install GLab.GLab
                </pre>
                <pre className="rounded bg-muted px-2 py-1 text-xs mt-1">
                  scoop install glab
                </pre>
              </div>

              <div>
                <p className="text-sm text-muted-foreground">
                  {t('settings:integrations.gitlab.cliSetup.install.linux')}{' '}
                  <a
                    href="https://gitlab.com/gitlab-org/cli#installation"
                    target="_blank"
                    rel="noreferrer"
                    className="underline"
                  >
                    {t(
                      'settings:integrations.gitlab.cliSetup.install.linuxSeeLink'
                    )}
                  </a>
                </p>
              </div>
            </div>
          </div>

          <div className="space-y-3">
            <p className="text-sm font-medium">
              {t('settings:integrations.gitlab.cliSetup.authenticateTitle')}
            </p>

            <div className="space-y-2 ml-2">
              <div>
                <p className="text-sm text-muted-foreground">
                  {t(
                    'settings:integrations.gitlab.cliSetup.authenticate.gitlabCom'
                  )}
                </p>
                <pre className="rounded bg-muted px-2 py-1 text-xs">
                  glab auth login
                </pre>
              </div>

              <div>
                <p className="text-sm text-muted-foreground">
                  {t(
                    'settings:integrations.gitlab.cliSetup.authenticate.selfHosted'
                  )}
                </p>
                <pre className="rounded bg-muted px-2 py-1 text-xs">
                  glab auth login --hostname gitlab.mycompany.com
                </pre>
              </div>
            </div>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            {t('common:buttons.close')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
});

export const GlabCliSetupDialog = defineModal<void, void>(
  GlabCliSetupDialogImpl
);
