import { formatDate, formatCurrency, capitalize, truncate } from "../../../basic-project/src/utils/format";

const today = new Date();
console.log(formatDate(today));

const price = formatCurrency(19.99);
console.log(price);
