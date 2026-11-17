// 来源：rtugeek/utilities 固定提交3d733215bac3e9c1e58125518f100c1601650828，src/api/FinanceApi.ts
export const CNY = {
    code: 'CNY',
    name: '人民币',
    symbol: '¥',
    flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/cn.svg',
};
export const USD = {
    code: 'USD',
    name: '美元',
    symbol: '$',
    flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/us.svg',
};
export const SoutheastAsiaCurrencyMapping = [
    {
        code: 'PHP',
        name: '菲律宾比索',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/php.svg',
        symbol: '₱',
        sortName: '比索',
    },
    {
        code: 'SGD',
        name: '新加坡元',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/sgd.svg',
        symbol: 'S$',
    },
    {
        code: 'MYR',
        name: '马来西亚林吉特',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/myr.svg',
        symbol: 'RM',
        sortName: '林吉特',
    },
    {
        code: 'THB',
        name: '泰铢',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/thb.svg',
        symbol: '฿',
    },
    {
        code: 'VND',
        name: '越南盾',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/vnd.svg',
        symbol: '₫',
    },
    {
        code: 'IDR',
        name: '印尼盾',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/idr.svg',
        symbol: 'Rp',
    },
];
export const PopularCurrencyMapping = [
    CNY,
    USD,
    {
        code: 'JPY',
        name: '日元',
        symbol: '¥',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/jp.svg',
    },
    {
        code: 'EUR',
        name: '欧元',
        symbol: '€',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/eu.svg',
    },
    {
        code: 'HKD',
        name: '港币',
        symbol: '$',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/hk.svg',
    },
    {
        code: 'KRW',
        name: '韩元',
        symbol: '₩',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/kr.svg',
    },
    {
        code: 'AUD',
        name: '澳元',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/au.svg',
        symbol: 'A$',
    },
    {
        name: '卢布',
        code: 'RUB',
        symbol: '₽',
        flag: 'https://assets.msn.cn/weathermapdata/1/static/finance/taskbar/countryflag/ru.svg',
    },
];
export const CurrencyMapping = [
    ...PopularCurrencyMapping,
    ...SoutheastAsiaCurrencyMapping,
];
