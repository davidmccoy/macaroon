// Absolute minimal Roon extension test
const RoonApi = require('node-roon-api');

// Capture any unhandled errors
process.on('uncaughtException', (err) => {
    console.error('UNCAUGHT EXCEPTION:', err);
});

process.on('unhandledRejection', (reason, promise) => {
    console.error('UNHANDLED REJECTION:', reason);
});

console.log('Creating minimal Roon extension...');

const roon = new RoonApi({
    extension_id: 'com.test.minimal',
    display_name: "Minimal Test",
    display_version: "1.0.0",
    publisher: 'David McCoy',
    email: 'REDACTED_EMAIL',
    website: 'REDACTED_WEBSITE',

    core_paired: function(core) {
        console.log('*** CORE PAIRED! ***', core.display_name);
    },

    core_unpaired: function(core) {
        console.log('*** CORE UNPAIRED! ***', core.display_name);
    },

    log_level: 'all'
});

console.log('Initializing services...');
roon.init_services({});

console.log('Starting discovery...');
roon.start_discovery();

console.log('Extension running. Press Ctrl+C to exit.');
console.log('Check Roon Settings → Extensions to see if "Minimal Test" appears.');
