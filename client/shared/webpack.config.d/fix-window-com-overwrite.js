/**
 * 修复：阻止 h5App.js 覆盖 window.com 命名空间（桥接丢失）。
 *
 * 症状（本项目实测）：
 *   页面白屏，控制台报
 *     registerCallNative error, reason is: TypeError: Cannot read properties of undefined (reading 'registerCallNative')
 *     Uncaught ReferenceError: callNative is not defined
 *   运行时诊断 `window.com.tencent.kuikly.core.nvi`：nativevue2.js 加载后为 true，h5App.js 加载后变成 false。
 *
 * 原因：
 *   nativevue2.js（业务包）先加载，建立 window.com.tencent.kuikly.core.nvi（宿主与页面互调的桥）。
 *   随后 h5App.js（UMD）加载时执行 `globalThis['KuiklyUI:h5App'] = factory(...)`，其返回值里也带一份
 *   `com` 命名空间，UMD wrapper 直接**整体赋值**到 window.com，把先前建立的 nvi 一并冲掉。
 *
 * 解法：
 *   把 window.com 改成带 deep-merge 的访问器属性。后发生的赋值不再覆盖，而是递归并入已有对象，
 *   于是 h5App 自己的 com 内容照常挂上，nativevue2 建立的 nvi 也得以保留。
 *
 * 注入目标：
 *   必须在**先执行**的 bundle 里注入才拦得住后续赋值。官方 demo 用分包模式的 `runtime.*.js`
 *   （分包时它最先执行）；本项目是单 bundle 模式，最先执行的是 `nativevue2.js`，故两者都匹配。
 */

const webpack = require('webpack');

console.log('[fix-window-com-overwrite] webpack patch 已加载');

const deepMergeCode = `
(function() {
    // 兼容小程序环境：使用 global 而不是 window
    var _global = typeof global !== 'undefined' ? global : (typeof window !== 'undefined' ? window : this);
    var _com = _global.com;

    function deepMerge(target, source) {
        if (!source || typeof source !== 'object') return target;
        if (!target || typeof target !== 'object') return source;

        var keys = Object.keys(source);
        for (var i = 0; i < keys.length; i++) {
            var key = keys[i];
            var targetVal = target[key];
            var sourceVal = source[key];

            if (targetVal && sourceVal &&
                typeof targetVal === 'object' && typeof sourceVal === 'object' &&
                !Array.isArray(targetVal) && !Array.isArray(sourceVal)) {
                deepMerge(targetVal, sourceVal);
            } else if (!(key in target)) {
                target[key] = sourceVal;
            }
        }
        return target;
    }

    Object.defineProperty(_global, 'com', {
        get: function() { return _com; },
        set: function(val) {
            if (_com && val && typeof _com === 'object' && typeof val === 'object') {
                deepMerge(_com, val);
            } else {
                _com = val;
            }
        },
        configurable: true
    });
})();
`;

config.plugins.push(
    new webpack.BannerPlugin({
        banner: deepMergeCode,
        raw: true,
        entryOnly: false,
    })
);
