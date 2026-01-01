// Quick Setup Wizard - Main exports
export { QuickSetupWizard, type QuickSetupConfig } from './QuickSetupWizard';
export { QuickSetupModal } from './QuickSetupModal';

// Components
export { CollapsibleSection } from './components/CollapsibleSection';
export { QuickSetupProviderCard, ProviderCardGrid } from './components/ProviderCard';
export { StepIndicator, quickSetupSteps, type Step } from './components/StepIndicator';

// Steps
export { ProviderSelect } from './steps/ProviderSelect';
export { CredentialsForm, type CredentialsData } from './steps/CredentialsForm';
export { ModelSelect } from './steps/ModelSelect';
export { CapabilityConfirm, type ModelCapabilities, getDefaultCapabilities } from './steps/CapabilityConfirm';

// Services
export * from './services/quickSetupApi';

// Data
export {
  internationalProviders,
  chinaProviders,
  proxyProviders,
  allProviders,
  getProviderById,
  getAllProviders,
  modelBaseTypes,
  matchModelBaseType,
  type ProviderPreset,
  type ProviderCategory,
  type ModelPreset,
  type ModelBaseType,
  type ModelBaseTypeInfo,
} from './data/providerPresets';
