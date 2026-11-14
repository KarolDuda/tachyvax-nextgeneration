import { call } from './api.js';
export const S = { info: null, catalog: null, nav: null, lic: { premium: false, freeIds: [], hwid: '', status: { state: 'free' } }, go: () => {} };
export async function refreshInfo() { S.info = await call('app_info'); S.nav && S.nav.updateBars(); return S.info; }
export async function refreshLicense() { try { S.lic = await call('license_info'); } catch (e) { console.error(e); } S.nav && S.nav.updateNav && S.nav.updateNav(); return S.lic; }
export async function getCatalog() { return S.catalog || (S.catalog = await call('catalog_list')); }
export const isLocked = id => !S.lic.premium && !S.lic.freeIds.includes(id);
