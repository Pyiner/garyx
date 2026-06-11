import React from 'react';
import ReactDOM from 'react-dom/client';

import { I18nProvider } from '../i18n';
import { StorybookApp } from './StorybookApp';
import '../styles.css';
import './storybook.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <I18nProvider languagePreference="system">
      <StorybookApp />
    </I18nProvider>
  </React.StrictMode>,
);
