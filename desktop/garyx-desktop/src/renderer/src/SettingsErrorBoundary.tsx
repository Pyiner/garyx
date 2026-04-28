import { Component, type ErrorInfo, type ReactNode } from 'react';

type SettingsErrorBoundaryProps = {
  activeTab: string;
  onRetry: () => void;
  children: ReactNode;
};

type SettingsErrorBoundaryState = {
  error: Error | null;
};

export class SettingsErrorBoundary extends Component<
  SettingsErrorBoundaryProps,
  SettingsErrorBoundaryState
> {
  state: SettingsErrorBoundaryState = {
    error: null,
  };

  static getDerivedStateFromError(error: Error): SettingsErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Settings view crashed', error, info.componentStack);
  }

  componentDidUpdate(prevProps: SettingsErrorBoundaryProps) {
    if (prevProps.activeTab !== this.props.activeTab && this.state.error) {
      this.setState({ error: null });
    }
  }

  private handleRetry = () => {
    this.setState({ error: null });
    this.props.onRetry();
  };

  render() {
    if (!this.state.error) {
      return this.props.children;
    }

    return (
      <section className="panel settings-section">
        <div className="panel-header settings-section-header">
          <div className="settings-section-copy">
            <span className="eyebrow">Settings Error</span>
            <h3 className="settings-section-title">The current settings tab failed to render</h3>
            <p className="small-note">
              Garyx kept the rest of the app alive. Reload this tab or switch to another one.
            </p>
          </div>
        </div>
        <div className="settings-section-body">
          <div className="settings-surface-group">
            <div className="settings-surface-list">
              <div className="settings-control-row stacked">
                <div className="settings-control-row-copy">
                  <div className="settings-control-row-label">Error</div>
                  <p className="settings-control-row-description">
                    {this.state.error.message || 'Unknown renderer error'}
                  </p>
                </div>
                <div className="settings-control-row-control">
                  <button className="primary-button" onClick={this.handleRetry} type="button">
                    Reload Tab
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>
    );
  }
}
