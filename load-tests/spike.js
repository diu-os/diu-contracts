// Load Test: Spike — University Lecture Scenario
// QAR Q-04: < 5% errors при 100 одновременных пользователях
// Сценарий: лектор открывает симуляцию перед аудиторией (100 студентов одновременно)
// Запуск: k6 run load-tests/spike.js
// Результаты: load-tests/results/YYYY-MM-DD-spike.json (git-ignored)

import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  stages: [
    { duration: '30s', target: 100 },  // резкий всплеск (студенты заходят одновременно)
    { duration: '5m',  target: 100 },  // sustained (длительность лекции)
    { duration: '30s', target: 0 },    // ramp-down
  ],
  thresholds: {
    http_req_duration: ['p(95)<5000'],  // допустимо до 5s при пиковой нагрузке
    http_req_failed: ['rate<0.10'],     // не более 10% ошибок
  },
};

export default function () {
  const res = http.get('https://physics.diu-os.org');
  check(res, {
    'status 200': (r) => r.status === 200,
    'loads in 5s': (r) => r.timings.duration < 5000,
  });
  sleep(1);
}
