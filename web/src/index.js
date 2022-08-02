import { start } from '../pkg';
import './main.scss';

start();

window.addEventListener("hashchange", () => location.reload());
