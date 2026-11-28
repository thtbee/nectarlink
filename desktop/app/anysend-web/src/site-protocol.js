// SPDX-License-Identifier: GPL-3.0-or-later
import {utf8} from './bytes.js';

export const siteMarker = 'name="anysend-site" content="2"';
export const siteClaim = utf8.encode('AnySend browser v2');
export const siteAck = utf8.encode('AnySend sender v2');
export const fileKey = utf8.encode('AnySend file key v2');
export const lanProbe = utf8.encode('AnySend LAN probe v2');
