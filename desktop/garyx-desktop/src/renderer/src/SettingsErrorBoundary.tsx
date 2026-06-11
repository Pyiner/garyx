import { Component, type ErrorInfo, type ReactNode } from 'react';
import { I18nConsumer } from './i18n';

type SettingsErrorBoundaryProps = {
  activeTab: string;
  onRetry: () => void;
  children: ReactNode;
};

type SettingsErrorBoundaryState = {
  error: Error | null;
  componentStack: string | null;
};

/// A lazily loaded settings chunk that no longer matches the running
/// renderer — the app bundle was replaced on disk (rebuild/update) while
/// this instance kept running. Only a relaunch loads a consistent bundle.
function isStaleBundleError(message: string): boolean {
  return (
    message.includes('already been declared') ||
    message.includes('dynamically imported module') ||
    message.includes('Importing a module script failed')
  );
}

export class SettingsErrorBoundary extends Component<
  SettingsErrorBoundaryProps,
  SettingsErrorBoundaryState
> {
  state: SettingsErrorBoundaryState = {
    error: null,
    componentStack: null,
  };

  static getDerivedStateFromError(error: Error): Partial<SettingsErrorBoundaryState> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Settings view crashed', error, info.componentStack);
    this.setState({ componentStack: info.componentStack ?? null });
  }

  componentDidUpdate(prevProps: SettingsErrorBoundaryProps) {
    if (prevProps.activeTab !== this.props.activeTab && this.state.error) {
      this.setState({ error: null, componentStack: null });
    }
  }

  private handleRetry = () => {
    this.setState({ error: null, componentStack: null });
    this.props.onRetry();
  };

  render() {
    if (!this.state.error) {
      return this.props.children;
    }
    const errorMessage = this.state.error.message;

    return (
      <I18nConsumer>
        {({ t }) => (
          <section className="panel settings-section">
            <div className="panel-header settings-section-header">
              <div className="settings-section-copy">
                <span className="eyebrow">{t('Settings Error')}</span>
                <h3 className="settings-section-title">{t('The current settings tab failed to render')}</h3>
                <p className="small-note">
                  {t('Garyx kept the rest of the app alive. Reload this tab or switch to another one.')}
                </p>
              </div>
            </div>
            <div className="settings-section-body">
              <div className="settings-surface-group">
                <div className="settings-surface-list">
                  <div className="settings-control-row stacked">
                    <div className="settings-control-row-copy">
                      <div className="settings-control-row-label">{t('Error')}</div>
                      <p className="settings-control-row-description">
                        {errorMessage || t('Unknown renderer error')}
                      </p>
                      {errorMessage && isStaleBundleError(errorMessage) ? (
                        <p className="settings-control-row-description">
                          {t('The app files were updated while Garyx was running. Quit and reopen Garyx to load the new version.')}
                        </p>
                      ) : null}
                      {this.state.componentStack ? (
                        <details className="settings-error-stack">
                          <summary>{t('Technical details')}</summary>
                          <pre>{this.state.componentStack.trim()}</pre>
                        </details>
                      ) : null}
                    </div>
                    <div className="settings-control-row-control">
                      <button className="primary-button" onClick={this.handleRetry} type="button">
                        {t('Reload Tab')}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </section>
        )}
      </I18nConsumer>
    );
  }
}
