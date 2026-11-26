import React from 'react';
import {AppRegistry} from 'react-native';
import {App} from './App';
import {name} from './app.json';
// Ephemeral process identifier: no device identifier or credential persistence.
const deviceId = 'bluefin_vega_' + Date.now().toString(36) + Math.random().toString(36).slice(2);
AppRegistry.registerComponent(name, () => () => <App deviceId={deviceId} />);
