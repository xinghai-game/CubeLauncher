<script lang="ts">
  import { onMount } from 'svelte';
  import { revealPage } from './lib/motion';
  import { ui } from './lib/state.svelte';
  import { bootstrap, connectEvents, refreshCatalog } from './lib/actions';
  import TitleBar from './components/TitleBar.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import HomePage from './components/HomePage.svelte';
  import AccountsPage from './components/AccountsPage.svelte';
  import AccountPage from './components/AccountPage.svelte';
  import InstancesPage from './components/InstancesPage.svelte';
  import InstancePage from './components/InstancePage.svelte';
  import VersionCatalogPage from './components/VersionCatalogPage.svelte';
  import InstallWizard from './components/InstallWizard.svelte';
  import InstallProgressPage from './components/InstallProgressPage.svelte';
  import ImportPage from './components/ImportPage.svelte';
  import SettingsPage from './components/SettingsPage.svelte';
  import SettingsSectionPage from './components/SettingsSectionPage.svelte';
  import Overlays from './components/Overlays.svelte';
  import AccountModal from './components/AccountModal.svelte';
  import DeleteConfirmModal from './components/DeleteConfirmModal.svelte';

  onMount(() => {
    let unsubscribe = () => {};
    void (async () => {
      // A failing event subscription must not stop the launcher from loading data.
      unsubscribe = await connectEvents();
      await bootstrap();
      // The version list is fetched after the first paint: a fresh cache answers
      // with one disk read, and a stale one is refreshed in the background.
      void refreshCatalog(false);
    })();
    return () => unsubscribe();
  });
</script>

<svelte:head><title>CubeLauncher · Minecraft Java 启动器</title></svelte:head>

<div class="app-shell">
  <TitleBar />
  <div class="body">
    <Sidebar />
    <main class="main-content">
      <!-- `.page` is the centred column: the window itself never scrolls, so this
           element owns the (rare) overflow instead of the document. -->
      <div class="page" use:revealPage={ui.route.name + (ui.route.name === 'settings' ? `:${ui.route.section}` : ui.route.name === 'wizard' ? `:${ui.route.step}` : ui.route.name === 'instance' ? `:${ui.route.instanceId}:${ui.route.tab}` : ui.route.name === 'account' ? `:${ui.route.accountId}` : '')}>
        {#if ui.route.name === 'home'}
          <HomePage />
        {:else if ui.route.name === 'accounts'}
          <AccountsPage />
        {:else if ui.route.name === 'account'}
          <AccountPage />
        {:else if ui.route.name === 'instances'}
          <InstancesPage />
        {:else if ui.route.name === 'instance'}
          <InstancePage />
        {:else if ui.route.name === 'versions'}
          <VersionCatalogPage />
        {:else if ui.route.name === 'wizard'}
          <InstallWizard />
        {:else if ui.route.name === 'install'}
          <InstallProgressPage />
        {:else if ui.route.name === 'import'}
          <ImportPage />
        {:else if ui.route.section}
          <SettingsSectionPage />
        {:else}
          <SettingsPage />
        {/if}
      </div>
    </main>
  </div>
</div>

<Overlays />

{#if ui.showAccount}<AccountModal />{/if}
{#if ui.showDeleteConfirm}<DeleteConfirmModal />{/if}
