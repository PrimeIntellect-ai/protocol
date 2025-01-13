import { Etcd3 } from 'etcd3';

async function main() {
  const client = new Etcd3({
    hosts: process.env.ETCD_HOSTS?.split(',') || ['http://etcd:2379']
  });

  try {
    // Test etcd connection
    await client.put('test-key').value('test');
    const value = await client.get('test-key').string();
    console.log('Retrieved from etcd:', value);
  } catch (error) {
    console.error('Error:', error);
    process.exit(1);
  }
}

main();